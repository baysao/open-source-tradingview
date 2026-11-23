---
phase: 01-rust-parity
plan: 01
status: UNIFIED
applied: 2026-11-23
unified: 2026-11-23
type: Summary
about: "vtradingview"
---

# 01-01 SUMMARY: workspace, contract types, static rule, market routes, G1 suite

## Result

All three tasks done; all five acceptance criteria met. The G1 oracle runs against Python from
this plan: **18 of 18 contract cases pass** against the running Python hub, and `/api/instruments`
matches Python **value for value (979 instruments)** when built from Python's own symbols.

## Tasks

| Task | Status | Evidence |
|---|---|---|
| 1. Workspace, contract types, Config, AppState skeleton, static rule | done | `cargo build`, `clippy -D warnings`, `fmt --check` clean; static table test passes |
| 2. Provider REST halves, Catalog, BarsCache, market routes | done | 22 unit tests pass (ticker, Bybit ticker, Deriv windows and digits, instruments keys and ranks, BarsCache single-flight and TTL, errors not cached, config) |
| 3. G1 contract suite | done | `contract: 18 passed, 0 skipped, 0 failed`; mutation test bites (below) |

## Acceptance criteria

| AC | Result | Measured |
|---|---|---|
| AC-1 workspace builds clean | pass | build, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` exit 0; 0 build warnings; `Cargo.lock` generated; `rust-toolchain.toml` pins 1.95.0 |
| AC-2 static rule, HEAD closed | pass | unit table (6 allowed, 12 refused); contract probes `GET /%2e%2e/LICENSE`, `GET /a%5cb.js`, `GET /vendor/a/b.js`, `GET /server.py`, `HEAD /.env` all 404; `/` and `/hub.html` 200 |
| AC-3 market routes keep the contract | pass | status, symbols, instruments, 5 bars cases (Deriv, Binance, Bybit, scroll-back, unknown source) and the `tf=M2` 502 `'M2'` case all match Python's status and JSON shape; bar counts equal `limit` |
| AC-4 suite cannot pass silently | pass | with `CONTRACT_PY_URL` unset: `contract: skipped: CONTRACT_PY_URL unset (18 cases not run)` and `0 passed, 18 skipped, 0 failed` (cargo still lists the test function as ok; the suite's own line says skipped) |
| AC-5 BarsCache single-flight and TTL | pass | 8 concurrent gets make 1 fetch; newest window refetched after 6 s; scroll-back page served from cache after 6 s; `Fresh` refetches; 2 failing gets make 2 fetches (errors not cached); counters: served 12, upstream_calls 4 |

### Negative control (the oracle can fail)

Renaming `SymbolsResponse.arm` to `arm_mutated` made the suite report
`FAIL symbols: at "": keys only in Python ["arm"], only in Rust ["arm_mutated"]` and
`contract: 17 passed, 0 skipped, 1 failed`; the field was restored (grep count 0) and the suite
returned to 18 of 18.

## Decisions and facts recorded

- D-003 applied: `/api/symbols` sends `mt5: "off"` and `arm: "off"` (an `Off` type that can only
  serialize "off"); `ws_port` is the Rust server's own port.
- Python's full symbol list (43 Deriv, 3,403 Bybit, 9 Binance) deserializes into the strict Rust
  model (`deny_unknown_fields`, one struct per source shape) with no error.
- The instruments latent bug (Python caches by total count only) is fixed by rebuilding at every
  catalog write; no contract change.

## Deviations (auto-fixed or chosen during APPLY, each verified)

1. **Crate versions.** The plan said "latest release on the day". The latest releases (tower-http
   0.7, reqwest 0.13, tokio-tungstenite 0.30, lru 0.19, toml 1.x) were not all confirmed compatible
   with axum 0.8, so the workspace pins the versions that are: tower-http 0.6, reqwest 0.12
   (rustls), tokio-tungstenite 0.26, lru 0.12, toml 0.8; `Cargo.lock` is the pin. Verified by a clean
   build and clippy.
2. **`AppState::build` is synchronous** (the plan's Interfaces said `async`): it does no I/O, so
   async added nothing. `AppState` also holds a private `sources: Arc<Sources>` for the refresh loops;
   the public fields are the planned `cfg`, `catalog`, `bars`.
3. **Instruments ordering test** runs as an integration test against Python
   (`instruments_match_python_exactly`) instead of a committed fixture: a full fixture would be
   about 700 KB and go stale; the live comparison is stronger (exact values, 979 instruments).
4. **BarsCache clock** uses `tokio::time::Instant` (equal to std at run time) so the TTL is testable
   with the paused test clock; found when the first TTL test failed.
5. **Binance klines** are parsed as JSON arrays by index (the first tuple-struct version would
   reject Binance's 12-field rows); found in review before the contract run.
6. **Oracle process.** Starting `python hub/server.py` failed because port 8765 was already taken
   by an existing `python hub/server.py` (PID 21944) from this repository, serving 8770. That
   instance runs the unchanged `hub/` code (`git diff main -- hub/` is empty), so it was used as
   `CONTRACT_PY_URL` and left running.

## Skill audit

- `pstack_stage: plan; APPLY ran in-session` (architect ran at plan time; its sketch is
  `01-ARCHITECTURE.md`).
- `/paul-security` (optional): not run for the static rule; the probe cases cover traversal,
  backslash, dotfiles, nested vendor paths and HEAD.

## Deferred

None. No debt rows written.

## Files

Created: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `crates/hub-types/{Cargo.toml,
src/lib.rs, src/market.rs}`, `crates/hub-core/{Cargo.toml, src/lib.rs, src/config.rs,
src/static_files.rs, src/catalog.rs, src/bars_cache.rs, src/market/{mod,ticker,bars_query,instruments}.rs,
src/sources/{mod,binance,bybit,deriv}.rs, src/routes/{mod,market}.rs, tests/contract.rs,
tests/contract/requests.toml}`. Modified: `.gitignore`.

## Next-plan readiness (01-02)

Ready. 01-02 imports `AppState`, `Endpoints` (add live URLs), `BarsCache::get` with `Freshness`,
`Ticker`, `Source`, `Tf`, `Bar`, and appends `/ws` and `/api/health` cases to `requests.toml`.

## UNIFY reconciliation (2026-11-23)

- **Plan against actual:** all three tasks and AC-1 to AC-5 hold. Gates re-run at UNIFY from bare
  commands: `cargo fmt --all --check` exit 0, `cargo clippy --workspace --all-targets -- -D warnings`
  exit 0, `cargo test --workspace` exit 0. The contract suite results above were produced this
  session (18/18 and 979 instruments); they need the Python hub, so they were not re-run here.
- **Repository against SUMMARY:** `git status` shows every 01-01 file untracked or modified and no
  commit since `0b620f7`; no file outside the plan's list was touched (`.gitignore` is in the list).
- **Deviations propagated backwards:** crate versions recorded as D-005 and written into spec item 1
  and the 01-01 plan; `AppState::build` is synchronous, amended in the 01-01 plan Interfaces; 01-02
  and 01-03 do not name the old signature.
- **AC text against reality:** no AC was amended; the AC-4 note (cargo lists a skipped run as ok, the
  suite's own line says skipped) is prose against reality, and the AC still holds.
- **Reflect and RSI:** deferred to phase close (reflect: pstack; 01-01 is not the last plan of
  Phase 01).
- **Skill audit:** `/architect` loaded at plan time (required, plan stage); `/paul-security`
  optional and not run (unverifiable, warn-only); no phase override names Phase 01.
- **Close-out writes:** rules: +0, decisions: +1 (D-005). Learnings: +3 added (L-001 to L-003),
  0 bumped, 0 promoted, 0 removed. Debt: +1 added (DEBT-001, artifact), 0 discharged.

### Deferred Items (generated from `debt.py --run-checks`)

| Id | Status | Title | Trigger |
|---|---|---|---|
| DEBT-001 | waiting | Commit the Phase 01 work (crates/, Cargo files, .paul/ planning and 01-01 SUMMARY) on arch/axum-tauri | event:first-commit |

**Open risk: UNIFIED is not LANDED.** Every file of this plan is uncommitted until the owner
commits; DEBT-001 checks it against `HEAD`.
