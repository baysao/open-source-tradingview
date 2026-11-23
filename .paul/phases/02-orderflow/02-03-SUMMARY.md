---
phase: 02-orderflow
plan: 03
status: UNIFIED
applied: 2026-11-23
unified: 2026-11-23
type: Summary
about: "vtradingview"
---

# 02-03 SUMMARY: the order-flow routes, the contract against Python, the browser check

## Result

`hub-server` answers `/api/orderflow/delta`, `/profile` and `/footprint` like the Python hub. Shapes
and status codes match, the error texts are identical, and on closed windows every value matches
with no float allowance needed. The hub's order-flow chart types work in the browser on the Rust
server: volume footprint, session volume profile and the delta indicator, on Binance BTCUSD and
Deriv gold.

## Tasks

| Task | Status | Qualify |
|---|---|---|
| 1. Route handler, AppState wiring, in-process route tests | DONE | PASS: `orderflow_routes.rs` |
| 2. Contract cases against the Python hub | DONE_WITH_CONCERNS | PASS: 40/0/0. Concern resolved: one case was machine-dependent and was removed (deviation 3) |
| 3. checkpoint:human-verify | agent-run (R-6) | Verified by agent with Playwright, delegated by the owner under standing rule R-6; the residual owner look is DEBT-008 |

## Acceptance criteria

| AC | Result | Evidence |
|---|---|---|
| AC-1 routes answer like Python | pass | In-process against a local fake. Delta, profile and footprint return 200 with Python's keys in Python's order. 502 bodies: `KeyError: 'M2'`, `ValueError: invalid literal for int() with base 10: 'abc'`, `ValueError: no real buyer/seller data for BYBIT_MNTUSDT`, `RuntimeError: mt5 history is not available in this server (D-001)` (profile) and `arm ...` (footprint), and the two D-009 guards (`rows must be 100000 or fewer`, `row_size and va must be finite numbers`). `feed=mt5` on a real-source delta returns 200. Every response carries `Cache-Control: no-store`. Each error writes exactly one warn line (`orderflow delta BTCUSD M2: 'M2'`, `orderflow profile BTCUSD None: mt5 ...`) in `/api/logs`. |
| AC-2 contract against Python | pass | `contract: 40 passed, 0 skipped, 0 failed` with the Python hub on 8770 (the Rust router in-process); 979 instruments match exactly. 15 order-flow cases: 8 closed-window cases compared by value (no float differed, so the 1e-4 allowance was never used), 3 live cases compared by shape and status, and 4 error cases compared by text. Mutation: changing the `unit` text failed 3 named cases (applied, run, restored and touched); the clean run is 40/0/0 again. |
| AC-3 chart types in the browser (agent-run) | pass | hub-server on 8771 under gaffa, Playwright. BTCUSD Binance: the footprint shows "REAL trades, Binance BTCUSDT, row 25" with per-row buy/sell, POC and the delta and total box; the session volume profile shows per-session histograms with POC and value-area lines; the delta pane shows real taker buy minus sell. Deriv gold: the footprint shows "REAL trades, Binance XAUUSDT, row 0.5", shifted onto the Deriv price (gold maps to XAUUSDT, not PAXG: R32). Over one clean minute: 19 footprint and 11 delta refreshes, all 200, no 429, no console error beyond the known `favicon.ico` 404 (Python also 404), and no order-flow error line since boot. Screenshots: `.playwright-mcp/of-footprint-btc.png`, `of-svp-delta-btc.png`, `of-footprint-xau.png`. |

Gates: `cargo fmt --all --check` 0, `cargo clippy --workspace --all-targets -- -D warnings` 0,
`cargo test --workspace` 0 (84 passed, 0 failed).

## Deviations and findings (each verified)

1. **Key order was not compared until this plan (correction to 02-01 and 02-02).**
   - The finding: `serde_json` was built without `preserve_order`, so every parsed object sorted its
     keys. The golden and replay comparators therefore checked sorted key sets, not Python's order.
     The 02-01 and 02-02 SUMMARYs claimed "the same key order"; that claim was not true when written.
     The first route test showed it.
   - The fix: `hub-core` enables `preserve_order` as a dev-dependency feature, so it applies to test
     builds only.
   - The proof: the golden and replay suites still pass, so our replies do match Python's order.
     Moving `source` ahead of `rows` in `DeltaReply` fails the golden suite on every delta case.
   - L-017.
2. **Row guards (D-009).** A profile over 1,000,000 rows, or a footprint candle or minute spanning
   over 1,000,000 price rows, answers 502 `ValueError`. `footprint` therefore returns a `Result`.
   `rows` above 100,000 is refused, and so is a non-finite `row_size` or `va` (Python writes the
   invalid JSON `NaN`). No answer Python can give changes.
3. **One contract case removed as machine-dependent.** On this PC `LOCAL_MT5` is set and a
   MetaTrader 5 terminal is logged in, so Python answers `feed=mt5` profile with MT5 candles (200).
   The Rust server has no MT5 by decision (D-001) and answers 502 (D-009). That Rust answer is tested
   in `orderflow_routes.rs`; the requests file says why the case is absent.
4. **A 429 during the browser check came from the Python hub, not the Rust one.** The contract run
   sent the Python hub a live BTC footprint, which started its own trade store, back-filling 30
   hours under its own 1,900-per-minute budget. Rust's store was loading too, and the two together
   exceeded Binance's 2,400 per minute for this IP.
   - Proof: with the Rust hub stopped, Binance's `X-MBX-USED-WEIGHT-1M` kept climbing (701, then
     1,482 in ten seconds), and it fell to 1 at 16:15:31, the moment Python's store went idle (600 s
     after its last use).
   - Rust's handling was right and matches Python: an upstream 429 becomes 502
     `HTTPError: HTTP Error 429: Too Many Requests` and starts the ban window.
   - After the restart, the clean minute above had no error.
   - In production one hub runs per IP. Side-by-side oracle runs share the IP's limit (L-018).

## Skill audit

- `/architect`: plan stage. `pstack_stage: plan; APPLY ran in-session`.
- `/paul-security` (optional, guidance):
  - Query parsing takes numbers only through `py_int`/`py_float`, and the boundary guards
    (deviation 2) bound memory.
  - The routes sit behind the existing `/api/*` rate limit (D-007) and security headers.
  - No new trust surface beyond the three read-only GET endpoints on public market data.
- R-6: the human-verify checkpoint was agent-run; residual owner row DEBT-008.

## Reflect and RSI

This is the last plan of Phase 02 (`reflect: pstack`). In Claude Code `/reflect` is user-only:
pending the owner typing `/reflect`, then `rsi`. Not yet run.

## Close-out

- rules: +0, decisions: +0 (D-009 covers the guards and the MT5 answer)
- learnings: +2 added (L-017, L-018), 0 bumped, 0 promoted, 0 removed
- debt: +1 added (DEBT-008, owner look), 0 discharged; DEBT-007 tightened to the 02-03 files

**Open risk: UNIFIED is not LANDED.** All Phase 02 files are uncommitted until the owner commits
(DEBT-007).

## Files

Created: `crates/hub-core/src/routes/orderflow.rs`, `crates/hub-core/tests/orderflow_routes.rs`.
Modified: `crates/hub-core/src/lib.rs` (`AppState.orderflow`, three routes),
`crates/hub-core/src/routes/mod.rs`, `crates/hub-core/src/orderflow/{mod,calc}.rs` (guards,
`footprint` returns `Result`), `crates/hub-core/tests/{contract.rs,contract/requests.toml,orderflow_golden.rs}`,
`crates/hub-core/Cargo.toml` (dev `serde_json` with `preserve_order`).

## Phase 02 exit

- Every fixture equals Python bit for bit, with key order: golden 25 calls and replay 25 calls,
  0 failed, 0 tolerance used. Store scenarios 7/7. Contract 40/0/0, with order-flow values equal
  on closed windows.
- The Rust hub serves order flow without numpy, pandas or pyarrow. `requirements.txt` keeps them for
  the Python oracle until Phase 04 retires it.
- Open: `/reflect` (owner), the commit (DEBT-007), the owner look (DEBT-008), the first CI run
  (DEBT-005).
