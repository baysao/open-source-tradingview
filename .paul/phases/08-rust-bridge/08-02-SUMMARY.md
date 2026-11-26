# Plan 08-02 Summary: the four-route surface, the PollerSet, the shell

**Plan:** `.paul/phases/08-rust-bridge/08-02-PLAN.md` (approved 2026-11-26, option 1)
**Applied:** 2026-11-26T18:05+07:00
**Status:** APPLIED — 2 of 2 tasks PASS, 40 crate tests green, workspace green and tauri-free

## What shipped

| File | Contents |
|------|----------|
| `crates/mt5-bridge/src/live.rs` | `PollerSet`: one `Arc<Entry{tx: watch::Sender<LiveSnapshot>, asked: AtomicI64}>` per (symbol, tf) in an `Arc<Mutex<HashMap>>`, get-or-spawn, `wait()` = read-or-await with the oracle's loop (re-touch `asked` on every wake, 1 s nap cap, error raise, deadline answer), runner polling `read_bars` + `last_tick` on the LIVE session every 250 ms (first window `max(sec*4, 10 d)`, later `sec*4+1 d`, oracle line 203), idle reaper removing its own entry, `LivePolicy{poll, idle, wait}` shrinkable in tests. |
| `crates/mt5-bridge/src/http.rs` | the only axum module: `bearer_gate` middleware (`got.len() == want.len()` + `subtle` `ct_eq` over the whole `Bearer <token>` value, oracle line 266-269), the four GET routes, 404 fallback behind the gate, `error_json` shapes, percent-decode `parse_qs` parity (`+` = space, `%XX`, invalid escape literal), hand-built JSON in frozen key order for `/health` and `/wait`, `Json(Vec<Bar>)` bare array for `/bars`. |
| `crates/mt5-bridge/src/main.rs` | the shell: exe-dir `bridge.env` via `dotenvy` (no-overwrite), `Config::from_env` with the oracle's defaults (127.0.0.1, 8797, `http://127.0.0.1:22346/mcp`, required token) and its exact refusal line, `.bridge_offset` load ("last measured") and rewrite on each accepted measurement, offset task (immediate, then 300 s / 15 s until first success, oracle log line `server time offset {±h.h} h`), three `Mcp` sessions (LIVE/HIST/MISC), startup line `native MT5 bridge on <bind>:<port>`, `axum::serve`. |
| `crates/mt5-bridge/Cargo.toml` | adds workspace-pinned `axum`, `tower`, `subtle`, `dotenvy` (all four now have users); dev `serde_json` `preserve_order` for the key-order tests. |

## AC results

| AC | Verdict | Evidence |
|----|---------|----------|
| AC-1 startup parity | PASS | `tests::a_missing_token_prints_the_oracles_line` (exact refusal text, exit 1 observed: `refusal exit=1`), `tests::the_defaults_match_the_python_oracle`, `tests::the_env_overrides_are_read`, `tests::the_offset_file_round_trips_or_falls_back`. Serve smoke: startup line `native MT5 bridge on 127.0.0.1:8797` printed. |
| AC-2 frozen route shapes | PASS | byte-exact asserts: 401 `{"error":"unauthorized"}` for missing/near-miss/lowercase-scheme (and the 404 fallback answers 401 signed-out); `/health` body `{"ok":true,"server":...,"pollers":...,"offset_s":...,"offset_src":...,"measured_at":...}` in order; `/bars` bare array `{"time","open","high","low","close","volume"}`; `/wait` `{"bar","key","tick_ms"}` in order; 404 `{"error":"not found"}`; 502 `{"error":"unknown timeframe M2"}`; downstream MCP error → 502 `{"error":"terminal is down"}`. Live curl smoke confirmed every gate on the real binary. |
| AC-3 PollerSet contract | PASS | `live::tests::one_poller_serves_every_waiter` (parked second waiter, call bound < 60, one entry), `a_waiter_on_the_current_key_blocks_until_it_changes`, `a_quiet_market_returns_at_the_deadline` (held to the deadline), `an_idle_poller_stops_and_leaves_the_map` (entry gone, no calls after), `http::tests::one_poller_serves_two_concurrent_waits`, `an_idle_entry_disappears_from_the_health_count` (`"pollers":1` → `"pollers":0`). |
| AC-4 errors never wedge | PASS | `a_poller_error_surfaces_as_502_with_the_text` (502 with the verbatim text; the next wait succeeds after the failure clears), `live::tests::a_terminal_error_reaches_the_waiter_and_does_not_wedge_it`. |

## Deviations (each with its reason)

1. **Query-error parity hardened beyond the plan text.** The oracle's `q["symbol"]` raises
   `KeyError` → 502 `{"error":"'symbol'"}`, and `int("5.5")`/`int("abc")` raise
   `invalid literal for int() with base 10: '5.5'`. The plan's AC-2 only named the shapes for
   the happy paths plus tf; the contract differ in 08-03 would have flagged these, so the routes
   now reproduce both texts exactly (verified by curl smoke).
2. **`/bars` limit/to parsing is strict `i64`, not `f64::trunc`.** Matching the oracle's `int()`
   (deviation 1's other half); a fractional limit is now an error, as Python produces.
3. **`/wait` route adds a 5 s headroom timeout** over the poller's own 20 s deadline
   (`WAIT_S + WAIT_HEADROOM_S`), mirroring the 07 contract's 25 s client budget; the plan did not
   name a route-level cap but the architecture's decision 4 does.
4. **Key order built by hand for `/health` and `/wait`** (and bar fields): `json!` preserves
   order only with the `preserve_order` feature, so it is a dev-dependency for the tests while
   the wire bodies are format!-assembled in contract order. `Json(Vec<Bar>)` for `/bars` is
   safe because `Bar`'s field declaration order is the contract order.
5. **Env tests serialized with a `static ENV_LOCK`** — the first full-suite runs were flaky
   (2 of 5 runs red) because `the_env_overrides_are_read` set `MT5_MCP_URL` while
   `the_defaults_match_the_python_oracle` read it in a parallel thread. One mutex; 10
   consecutive green runs after.

## Verification (bare exit statuses, R-8)

- `cargo test -p mt5-bridge` → **40 passed**, 0 failed (10 consecutive runs green after the env fix)
- `cargo test -p mt5-bridge -- --list` → 40 `: test` lines (L-026)
- `cargo test --workspace` → **200 passed, 1 ignored** (14 suites), exit 0; `grep -ci "tauri|webkit|wry"` → **0**
- `cargo fmt --all --check` → exit 0
- `cargo clippy -p mt5-bridge --all-targets -- -D warnings` → exit 0
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0
- Serve smoke on the built binary (dev box): startup line; 401 + `{"error":"unauthorized"}` signed-out; near-miss 401; 502 with `MT5_MCP_TOKEN is not set`/`terminal is down` on `/health`+`/bars`; 404 `{"error":"not found"}`; 502 `{"error":"unknown timeframe M2"}`; 502 `{"error":"'symbol'"}`; 502 `{"error":"invalid literal for int() with base 10: '5.5'"}`; no-token run prints the Python refusal line and exits 1.
- mcp.rs sibling-import grep still 0 (untouched by this plan, boundary held).

## Mutations (L-003/L-009: grepped landed, red run, grepped restored, green run)

| # | Mutation | Guard | Result |
|---|----------|-------|--------|
| m1 | bearer compare relaxed to `starts_with` | `http::tests::a_missing_or_near_miss_bearer_is_401_unauthorized` | FAILED (38/2 across suite) → restored |
| m2 | `wait()` returns the first snapshot without awaiting | 7 tests FAILED across live+http (`a_quiet_market_returns_at_the_deadline`, `a_waiter_on_the_current_key_blocks_until_it_changes`, `wait_answers_snapshot_key_tick_in_contract_order`, …) → restored |
| m3 | waiter stops re-touching `asked` | the deadline/idle tests FAILED → restored |
| m4 | limit clamp ceiling 5000 → 4000 | `term::tests::bars_trims_to_limit_and_clamps_at_five_thousand` FAILED → restored |
| m5 | poller get-or-spawn skips the lookup (double spawn) | initially UNOBSERVED (L-003: the guard was behind the test's own policy); `one_poller_serves_every_waiter` strengthened to park the second waiter and bound the call count, then FAILED → restored → green |
| m6 | bearer length check dropped (ct_eq over a truncated slice) | `a_missing_or_near_miss...` FAILED → restored |

One restored-mutation escape was caught and fixed: m1's first restore missed (the file still
carried `starts_with`); a later grep found it and the restore was re-applied before any green
run was trusted (L-009's lesson applied to restores, not just landings).

## Deferred

None. The contract suite (fake MCP terminal + differ) and the six phase-level mutations stay in
08-03 as planned.

## Next

08-03 (wave 2): the stdlib fake MCP terminal, `check-mt5c.sh` contract differ, the six named
mutations, `release.yml` windows leg + `mise.mt5.toml` + `host-install.ps1` revision, docs, and
the Python oracle's retirement after parity.

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack) — 08-02 is not the phase's last plan
- instruction edits proposed: none
- rsi: deferred to phase close (reflect: pstack)
