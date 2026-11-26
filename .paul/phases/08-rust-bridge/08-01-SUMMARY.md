# Plan 08-01 Summary: the crate, the sealed MCP client, the domain core

**Plan:** `.paul/phases/08-rust-bridge/08-01-PLAN.md` (approved 2026-11-26, option 1)
**Applied:** 2026-11-26T17:05+07:00
**Status:** APPLIED — 3 of 3 tasks PASS, 21 crate tests green, workspace green and tauri-free

## What shipped

| File | Contents |
|------|----------|
| `crates/mt5-bridge/Cargo.toml` | new root workspace member; only workspace-pinned deps (tokio, reqwest rustls, serde, serde_json float_roundtrip, chrono). No `subtle` or `dotenvy` yet — both arrive in 08-02 with the module that uses them (code auth, `bridge.env`). |
| `crates/mt5-bridge/src/main.rs` | stub: prints the startup line, exits 0; declares `mod mcp; mod term;` (08-02's shell replaces the body). |
| `crates/mt5-bridge/src/mcp.rs` | the sealed leaf: `Mcp::new(url, token)`, `call(name, args) -> Value`; initialize handshake (`2025-06-18`, `chart-bridge`/`1`), `Mcp-Session-Id` capture and resend, last-`data:`-line scan, `content[].text` join, `isError` verbatim, reopen-once on transport/HTTP/parse. No sibling import. |
| `crates/mt5-bridge/src/term.rs` | `Terminal` trait + `Terminals{live,hist,misc}`, `Tf`, `Bar`, `bar_key`, `py_str`, `iso`/`parse_time`, `tick_ms`, `bars` (resync loop, `SyncPolicy`), `last_tick`, `symbols`, `account_health`, `offset_measure`, `round_half_even`. |
| `Cargo.toml` | `crates/mt5-bridge` added to `[workspace] members`; workspace pins untouched (D-005). |

## AC results

| AC | Verdict | Evidence |
|----|---------|----------|
| AC-1 crate member, pinned deps | PASS | `cargo build -p mt5-bridge` exit 0; `cargo metadata` resolves the member; `Cargo.lock` gained 11 lines (the crate itself, no new transitive major). |
| AC-2 session lifecycle | PASS | `mcp::tests::handshake_captures_and_resends_the_session`: request order initialize → notifications/initialized → tools/call, `protocolVersion` `2025-06-18`, clientInfo `chart-bridge`/`1`, `Mcp-Session-Id: sess-1` on the tool call and absent on the first request, `Authorization: Bearer sekret`, `Accept: application/json, text/event-stream`. |
| AC-3 reopen-once and error parity | PASS | three reopen tests (`reopens_once_after_an_http_error`, `..._a_dropped_connection`, `..._an_unparseable_body`), each asserting initialize count == 2 and the retry succeeding with `sess-2`; `is_error_surfaces_the_text_verbatim`. |
| AC-4 both framings parse alike | PASS | `sse_and_plain_bodies_parse_the_same` (deep-equal Values) and `the_last_data_line_wins`. |
| AC-5 domain arithmetic | PASS | `py_str_matches_python_str_of_float` (18 measured pairs), `bar_key_uses_the_oracle_field_order_and_formatting`, `offset_rounds_half_to_even_and_refuses_a_stale_clock`, `tick_ms_slices_characters_20_to_23`, `tf_parses_exactly_the_seven_and_rejects_the_rest` (`unknown timeframe M2`), `bars_trims_to_limit_and_clamps_at_five_thousand` (99999 → 5000, span = 5000*60*1.5+4d), `bars_triples_the_span_while_the_answer_is_short`. |

## Deviations (each auto-fixed, with its verification)

1. **`{:.e}`/`{}` are not Python `str()` — the plan's Task 3 note was wrong.** Plan said "Rust `{}` on f64 is the same shortest-roundtrip; assert one known value". Measured: Rust prints `1829`, `10000000000000000`, `1e15`; Python prints `1829.0`, `1e+16`, `1000000000000000.0`. `/wait`'s `key` is diffed byte-for-byte (parity rule 4) and integral volumes are the common case, so `py_str` rebuilds Python's shape from Rust's shortest-roundtrip `{:e}` digits. Verified against 18 measured Python outputs plus inf/nan.
2. **`tick_ms` needed Python's clamping slice.** Rust's `str::get(20..23)` returns `None` when the stamp is short; Python's `s[20:23]` clamps. Real `time_ms` values from the terminal can be second-precision. Fixed (`min(len, 23)`); pinned by the `.12` → 12 ms and no-fraction cases.
3. **`round_half_even` written out, not `f64::round()`.** Python's `round()` is half-to-even; Rust's `round()` is half-away-from-zero. The tie is unreachable through the 90 s gate (a 900 s tie is refused), so it is a defensive parity unit test, and the test comment says so rather than implying the gate reaches it.
4. **`Terminal` trait instead of concrete `Mcp` in `term.rs`.** The plan named `Terminals{live,hist,misc}` over `Arc<Mcp>`; the functions take `&M: Terminal` so the domain tests inject in-memory sources (L-016: keep sockets out of tests) while production passes `Mcp`. Same public shape, no sockets under test.
5. **`subtle`/`dotenvy` not added in this plan.** The plan's AC-1 lists them, but neither has a user until 08-02 (constant-time bearer compare lives in `http.rs`; `bridge.env` loading lives in the shell). Adding an unused dep is the kind of "scaffolding for later" the repo forbids. Carried into 08-02.

## Verification (bare exit statuses, R-8)

- `cargo build -p mt5-bridge` → exit 0
- `cargo test -p mt5-bridge` → **21 passed**, 0 failed
- `cargo test -p mt5-bridge -- --list` → 21 `: test` lines (L-026: the filter's expected count pinned)
- `cargo test --workspace` → **181 passed, 1 ignored** (14 suites), exit 0; `grep -ci "tauri\|webkit\|wry"` → **0**
- `cargo fmt --all --check` → exit 0
- `cargo clippy -p mt5-bridge --all-targets -- -D warnings` → exit 0, no issues
- `cargo clippy --workspace --all-targets -- -D warnings` → exit 0, no issues
- `rg -c "^use crate::|use super::" crates/mt5-bridge/src/mcp.rs` → 0 (the leaf rule)
- distinct tool names in `term.rs` → 5 (`TOOLS` const + the five call sites)

## Mutations (L-003 / L-009: landed by grep, red run, restore by grep, green run)

| # | Mutation | Guard | Result |
|---|----------|-------|--------|
| m1 | `bar_key` format drops `volume` | `term::tests::bar_key` | FAILED → restored → 21 green |
| m2 | offset refusal widens `>= 90` → `>= 1800` | `term::tests::offset_rounds...` | FAILED → restored |
| m3 | limit clamp ceiling `5000` → `4000` | `term::tests::bars_trims...` | FAILED → restored |
| m4 | `Mcp-Session-Id` never captured | four `mcp::tests` (4 passed / 4 failed) | FAILED → restored |

Each mutation was grepped as landed before its run and grepped back to a count of 1 after restore.

## Deferred

None. Nothing in this plan revealed out-of-scope work; the contract suite's Python fake and the six
phase-level mutations stay in 08-03 as planned.

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack) — 08-01 is not the phase's last plan
- instruction edits proposed: none
- rsi: deferred to phase close (reflect: pstack)

## Next

08-02: `live.rs` (PollerSet over watch channels), `http.rs` (the only axum module; bearer compare
via `subtle`), `main.rs` (env + `bridge.env` via `dotenvy`, offset file + task, router, serve). It
depends on 08-01's `Terminal`/`Mcp`/`term` surface and adds no change to them.
