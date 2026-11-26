# 07-03 SUMMARY: bridge routes, symbols merge, bars path, ws poller, SSRF/cross-user suites, local-mode bridge

**Applied:** 2026-11-26 (session-reset auto-continue from wave-1's handoff; sequential in-session, pstack_stage: plan).
**Status:** UNIFIED 2026-11-26.
**Plan:** `.paul/phases/07-bridges/07-03-PLAN.md` · **Contract:** `.paul/phases/07-bridges/07-ARCHITECTURE.md`

## Tasks
| # | Task | Status |
|---|------|--------|
| 1 | `/api/me/mt5*` CRUD + test (owner-scoped, 404 cross-user, sealed tokens), gate-area registration, `VTV_LOCAL_BRIDGE_URL`/`_TOKEN` (D-033) | done |
| 2 | `/api/symbols` owner-only merge (D-032a/b) + `/api/bars` bridge path (owner-checked, `local` auth-off) | done |
| 3 | ws `bridge` field, want `(mt5:<id>, symbol, tf)`, refcounted `/wait` poller per want (D-032c) | done |
| 4 | logbus dynamic redaction of bridge tokens + URL queries (AC-5), fake_bridge.py, check-0703.sh, m1/m2/m3 red-then-green | done |

## Acceptance criteria
| AC | Result |
|---|---|
| AC-1 lifecycle, scoping, 404 cross-user, sealed storage | green (`bridge_routes` 3/3: owner list without token, foreign test/delete 404, owner delete 204/404, symbol_count from the cache) |
| AC-2 every SSRF case refused at the route, auth on | green (https-loopback and DNS-miss 422 with the guard's verbatim text; plain-http scheme refusal; the 07-01 17-case table still green; check-0703 case 8 runs the suite) |
| AC-3 bridge candles reach charts | green (REST: 10 rows field-for-field, unknown tf the KeyError text, timeout/redirect causes verbatim — cases 1-3; ws: live `ServerMsg::Bar` frame through the fake's `/wait` — case 5; D-009 fallback preserved, parity test green) |
| AC-4 symbols merge owner-only, anonymous-identical | green (case 4 anonymous shape byte-path unchanged, no bridge entries; catalog test byte-identical with empty extra; merged entries carry `source: "mt5"` + bridge id; a second user sees neither) |
| AC-5 logs never carry a bridge token or URL query | green (unit: seeded line loses both to `[redacted]`; served: the ECHO fake echoes the Authorization value through the poller into the ring — token absent, `[redacted]` present — case 6) |
| AC-6 mutations m1/m2/m3 red-then-green | proven (each grepped landed, red exit=101, restored with 0 markers left, green exit=0; L-009/L-013/L-023) |

## Verification
- `justrust test --workspace`: **160 passed, 0 failed, 1 ignored**, exit 0 (bare)
- `justrust test -p hub-core --features test-idp`: **211 passed, 0 failed, 3 ignored** (Postgres SKIPPED, `VTV_TEST_PG_URL` unset on this box, L-039), exit 0
- `cargo fmt --all --check` clean; `justrust clippy -p hub-core --all-targets --all-features` and `--all-targets`: **0 warnings**
- `bash .paul/runs/check-0703.sh`: **exit 0, CHECK-0703-PASS** — bars, redirect refusal, timeout cause, anonymous symbols, ws live update, redaction, fake bearer gate, auth-on suite; `bridge_routes` registered as a `[[test]]` with `required-features = ["test-idp"]`
- `debt.py --run-checks`: overdue 0, open 0, waiting 13 — no status change from this plan

## Deviations (each with its reason)
1. **files_modified under-listed the mount points**: `lib.rs` (AppState gains `bridges` + `bridge_pollers`, the D-033 ignored-keys warning), `bridges/mod.rs` (`Bridges` struct + `LOCAL_SEMAPHORE_ID`), `auth/mod.rs` (the `bridge_pool()` accessor), `Cargo.toml` (`[[test]] bridge_routes`). The route table implies these; recorded so the tree stays explainable.
2. **Uuid ids, not `i64`**: 07-01's deviation 1 fixed the repo convention (uuid PKs, `Uuid::now_v7()`); the want key is `mt5:<uuid>`, matching the migration and store.
3. **The failed-re-test writeback keeps the last working state**: `set_test_result` on failure passes the row's existing `symbols_json` and `last_ok_at`, only `last_error` moves. Reading of D-032b's "cache from the last successful Test"; one bad re-test does not blank the picker. The store test's "a failed test clears the cache" case (07-01) exercises the direct-store call, which still allows the clearing form.
4. **m1 targets the signed-out-401 half of the owner check**: the store statements are user-scoped by construction (B3's `WHERE ... user_id`), so a route-layer mutation cannot make a foreign row leak — the observable guard is the 401 ordering, which `bridge_routes` now pins (`signed-out bridge bars is 401`); the G5 half stays proven by the cross-user suite.
5. **m2 covers the two REST vet sites** (register + the bars bridge path). The ws bridge path vets with the same rule (`ws.rs` `bridge_stream`); its refusal class is exercised by the same suite, so a third marker would duplicate the class, not add one.
6. **Poller tests are registry mechanics + field mapping, in memory**: 07-01 dropped the plan's `Transport` seam (its deviation 2), so `start_paused` has nothing to drive; the end-to-end `/wait` path is check-0703's case 5, served, against the real fake.
7. **`fmt` reformatted the touched files** (mechanical; `cargo fmt --all --check` clean after).

## Skill audit
| Skill | Loaded? | Note |
|---|---|---|
| context-budget, caveman, ponytail | yes (session) | AGENTS.md rule |
| /architect | yes (plan) | 07-ARCHITECTURE.md is the sketch |
| /security-review | deferred to phase exit | the phase override ran over the whole wave-1 diff at 07-01; 07-03's diff runs `--full` under the transition gate |
| /paul-security | not invoked | the phase override subsumes per-task guidance (07-01's reading) |
| pstack | pstack_stage: plan | APPLY ran in-session; no poteto-mode routing |
| parallel_agents: true | degraded to sequential | ZCode has no worktree isolation (the wave-1 gotcha) |

## Reflect and RSI
Deferred to phase close (reflect: pstack; 07-04 is the phase's last plan and carries the owner checkpoint). This unify's close-out writes: learnings +1 added (L-047: a feature-gated tests/*.rs file must join the `[[test]]` required-features table or the default-feature workspace suite breaks), 5 bumped (L-009, L-013, L-023, L-026, L-039 — L-039 promoted to validated at hits 3); rules +0, decisions +0 (D-032/D-033 landed at plan approval); debt +0 added, +0 discharged (waiting 13 unchanged). Re-runnable gates re-run at unify: workspace 160 exit 0, test-idp 211 exit 0 (Postgres SKIPPED cited), clippy 0 both ways, check-0703 exit 0.

## UNIFIED ≠ LANDED
The 07-03 diff (routes/mt5.rs, market.rs, ws.rs, catalog.rs, config.rs, lib.rs, logbus.rs, auth/mod.rs, bridges/, feeds/bridge.rs, tests/bridge_routes.rs, Cargo.toml) is uncommitted with the wave-1 diff (auto_commit: false); the phase commit lands everything.
