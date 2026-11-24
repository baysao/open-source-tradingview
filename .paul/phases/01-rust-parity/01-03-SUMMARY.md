---
phase: 01-rust-parity
plan: 03
status: UNIFIED
applied: 2026-11-23
unified: 2026-11-23
type: Summary
about: "vtradingview"
---

# 01-03 SUMMARY: log bus, limits, hub-server, frontend switch, CI

## Result

The Rust `hub-server` now replaces `python hub/server.py` for the free feeds on one port. It serves
the existing hub unchanged, apart from `wsUrl()` and `restartChart()`, with the log panel, POST
guard, rate limit and security headers, and it runs under gaffa. Across all three chart engines,
three feeds, the 8-chart layout, Restart and the log panel, the browser check found one real
defect, a false silence alarm on new upstreams. It is fixed and covered by a regression test.

## Tasks

| Task | Status | Evidence |
|---|---|---|
| 1. Log ring, file sink, tracing layer, feed log, `/api/logs` | done | unit tests for redaction (expected strings from Python), throttle, ring caps, states, problems, file round trip, feed-log summary, silence and status edges; 7 contract cases |
| 2. Limits and headers, `hub-server`, Procfile, `dev.sh`, AGENTS.md | done | start-up lines, three AGENTS.md checks, burst test, header check, gaffa run (below) |
| 3. Frontend switch to `/ws`, CI | done | `wsUrl()` node check for both origins; `git diff --stat hub/` = `lab.js`, `hub.html` only; `ci.yml` parses (5 jobs) |
| 4. checkpoint:human-verify | verified by agent | Checkpoint: verified by agent with Playwright, delegated by the owner ("use playwright self check/verify"), 2026-11-23. Not an owner sign-off. |

## Acceptance criteria

| AC | Result | Measured |
|---|---|---|
| AC-1 `/api/logs` shape and POST guard | pass | contract vs Python: `logs`, `logs brief (no lines)`, `logs since=0`, journal 200 / 403 `forbidden origin` / 415 `JSON only` / 413 `too large`: 25 passed, 0 skipped, 0 failed. Mutation control: `journal_raw` renamed gives 3 FAIL; `JSON only` changed gives 1 FAIL; both restored, 25/25 again |
| AC-2 redaction and throttle | pass | `redaction_matches_python`: six inputs, expected strings captured from `hub/logbus.py redact()`; same `(src, msg)` inside `every = 30` is dropped, passes again after 30 s |
| AC-3 feed log once per upstream | pass | `summary_counts_once_per_upstream`: `updates=120 ticks/s=5.0`; `silence_raises_and_clears_the_series_problem`: problem `feed.binance:BTCUSDT M1` = `no data for over 120 s`, warn logged once, cleared when data returns |
| AC-4 binary starts like Python, one port | pass | within 15 s: `TradeX: open http://localhost:8771/hub.html  (data on ws://localhost:8771/ws)`, `Clock offset vs time.windows.com: +45529 ms` (Python measured +45524 ms on this PC, so the clock really is behind), `Deriv: 43 symbols loaded`, `Bybit: 3403 charts loaded`; only `127.0.0.1:8771` listening; `/api/status` all ok, 3 Deriv bars, `/.env` GET and HEAD 404; `nosniff` and `same-origin` headers present; 45 parallel `/api/status` gives 40 x 200 and 5 x 429 with `Retry-After: 1`, 200 again after 2 s. Port 8771 because the Python oracle holds 8770 |
| AC-5 existing UI on the Rust server | pass (agent-verified) | see "Browser verification" |
| AC-6 CI gates the phase | partial | `ci.yml` written (fmt, clippy, test, audit, contract with Python on 8770 and `hub-server` on 8771, skips and fails in the job summary); the contract command it runs gave 25/0/0 locally against the built binary. Not run on GitHub until the branch is pushed (DEBT-005); `cargo audit` is not installed locally, so it is unverified until that run |

## Browser verification (Playwright, hub-server under gaffa on 8771)

| Check | Result |
|---|---|
| Cold load | one socket `ws://localhost:8771/ws`, `history` then `bar` frames, 8 `/api/*` requests, 0 x 429 |
| Gold on Deriv, then Bybit | Deriv `frxXAUUSD` M1 853 bars live; Bybit `BYBIT_XAUUSDT` 1,000 bars live; Binance is not offered for gold, so it was checked on BTCUSD: `binance:BTCUSDT` 1,000 bars, 58 live bars in 10 s |
| 8-chart layout | 8 open sockets, all `/ws`, each with history and live bars; 11 `/api/*` requests, 0 x 429; `/api/health` shows one `deriv:frxXAUUSD` upstream for the 7 gold charts; screenshot `.playwright-mcp/verify-8charts.png` shows candles in all 8 |
| Restart chart | message `Reconnecting ...`, old sockets closed, one new socket with history and 60 bars, 0 calls to `/api/restart` |
| Engines | Lightweight, KLineChart, Vela: one socket each, 1,000 history bars, 55 to 61 live bars in 9 s |
| Log panel | start-up lines (with the previous run reloaded from `hub.log`), 60 s summaries per provider, upstream open/close lines, "No problems"; 46 `/api/logs` polls, 0 x 429; screenshot `.playwright-mcp/verify-logpanel.png` |
| Console | no MT5 warning. `favicon.ico` 404 (Python also 404). One Bybit older-history `502`, `error decoding response body`, not reproducible in 5 repeats on either server (1,000 bars, 200); Python also answers 502 on an upstream failure, so this is one bad Bybit reply, not a parity gap |

## Deviations (each verified)

1. **False silence alarm on a new upstream (found in the browser).** A just-opened upstream logged
   `frxXAUUSD M1: no data for 120 s` one second after `upstream opened`: `last_data` started as
   `None` and the silence rule read `None` as silent. Python starts `last_data` at stream start.
   `HubState::join` now sets `last_data = now`; regression test
   `new_upstream_counts_as_fresh_until_data`. After a restart on the fix, opening `ETHUSDT` logged
   no silence line (the two silence lines left in the panel are from the previous boot, dated before it).
2. **Status edges waited 30 s at boot.** The first status step ran on the first 2 s tick, before the
   catalog loaded, and logged `DOWN: 0 symbols`. Python first checks 30 s after start; `FeedLog::new`
   does the same, and the unit test covers the boot window.
3. **Tests wrote a stray log, and concurrent lines interleaved.** The default `hub/logs` is
   relative, so tests created `crates/hub-core/hub/logs/hub.log`, with two records on one line.
   Now only `AppState::build` (production) opens the file, `build_with` and the contract suite keep
   logs in memory, and `FileSink` writes line plus newline in one `write_all` under a mutex. Stray
   file deleted; a full test run leaves none.
4. **Rate limiter written by hand (D-007).** A per-IP token bucket in `net.rs` replaces the planned
   `tower_governor`. Same rules (20/s, burst 40, 429 with `Retry-After`, keyed by `client_ip`).
5. **`post_guard` returns `Box<Response>`** (clippy `result_large_err`); same statuses and bodies.
6. **`/modern-web-guidance` not retrieved.** The plan marks it required for Task 3. Both `npx` and
   the skill's fetch were denied in this session. The change uses only the URL API (Baseline
   widely available). Recorded as a skill gap (DEBT-004).
7. **Side-by-side runs need `HUB_LOG_DIR`.** Both servers default to `hub/logs/hub.log`, so the
   Rust hub ran with a scratchpad log dir next to Python; documented in AGENTS.md 1a.

## Known behaviour kept (not a regression)

- `lab.js` does not reopen a socket that closes. After a server restart the chart stays dead
  until Restart chart or a switch. Python behaves the same, and D-002 freezes this code. It matters
  for the hosted site, where every deploy restarts the server: DEBT-003 (Phase 04).
- With this branch's `lab.js`, the Python hub's own UI no longer gets live bars, because it has
  no `/ws` on 8770. The Python JSON API (the contract oracle) is unaffected.

## Skill audit

- `/architect`: plan stage, `01-ARCHITECTURE.md`. `pstack_stage: plan; APPLY ran in-session`.
- `/modern-web-guidance`: **gap** (required, not retrieved; deviation 6, DEBT-004).
- `/paul-security` (optional): not run; covered by the guard, limiter and `client_ip` tests.
  Note for Phase 04: from a loopback peer any local process can set `CF-Connecting-IP` and pick
  its rate-limit key; harmless in local mode, revisit when cloudflared is the loopback peer.
  Unverifiable (warn-only).
- `/technical-writing` (optional): not loaded; AGENTS.md edited directly. Unverifiable (warn-only).

## Reflect and RSI

Phase close (`reflect: pstack`): `/reflect` run by the owner on 2026-11-23. Panel of three reviewers
(judgment, tooling, divergent) on the session transcript plus a digest, then a synthesizer: 15
findings, 6 accepted, 5 rejected, 3 backlog. The owner approved all 6:

- A1 `paul-apply` checkpoint line and Step 3, and `plan-phase.md` step 4: a browser-observable
  human-verify is agent-run first.
- A2 `paul-apply` Step 3: a delegated checkpoint is recorded as `verified by agent, delegated by owner` with a
  residual owner row (DEBT-006 here).
- A3 `paul-plan` web guides: retrieve at plan time with the bare command; `unresolved (denied)` asks for the
  allowlist before APPLY.
- A4 to A6: learnings L-010 (idempotent edit scripts), L-011 (port init state from the oracle), L-012
  (Playwright state on `page`).

Owner answers: "use playwright self check/verify" is standing (Session Rule R-6); DEBT-003 stays at
Phase 04. Backlog (not skill edits): a heredoc-size hook or user-level instruction for Git Bash (15
failures this session). Rejected: per-plan commit asks under auto-advance (already covered by
UNIFIED-not-landed and DEBT-001), and making the D-002 frontend switch a decision checkpoint (already
decided). `rsi`: the new rows are candidates with hits 1; no experiment is owed (each validates on its
next occurrence), so no experiment debt rows.

## Close-out

- rules: +1 (R-6), decisions: +1 (D-007)
- learnings: +6 added (L-007 to L-012, three from the phase reflect), 1 bumped (L-003), 0 promoted, 0 removed
- debt: +4 added (DEBT-003 to DEBT-006), 1 discharged (DEBT-002), DEBT-001 tightened to 01-03 files

### Deferred Items (generated from `debt.py --run-checks`)

See the close report; the table is regenerated from the store, not hand-kept.

**Open risk: UNIFIED is not LANDED.** All Phase 01 files are uncommitted until the owner commits
(DEBT-001).

## Files

Created: `crates/hub-types/src/logs.rs`, `crates/hub-core/src/{logbus,feedlog}.rs`,
`crates/hub-core/src/routes/logs.rs`, `crates/hub-server/{Cargo.toml,src/main.rs}`, `Procfile`,
`scripts/dev.sh`, `.github/workflows/ci.yml`.
Modified: `Cargo.toml` (member, `hub-core`, `regex`, `dotenvy`), `crates/hub-core/Cargo.toml`,
`crates/hub-core/src/{lib.rs,net.rs,routes/mod.rs,feeds/state.rs}`, `crates/hub-types/src/lib.rs`,
`crates/hub-core/tests/{contract.rs,contract/requests.toml}`, `hub/lab.js`, `hub/hub.html`,
`AGENTS.md`, `.gitignore` (`.dev/`, `.playwright-mcp/`).

## Phase 01 exit

- Contract suite against Python and the built `hub-server`: 25 passed, 0 skipped, 0 failed (phase
  exit rule met locally); 979 instruments exact; ws and health shapes equal.
- `/.env` 404; one port; fmt, clippy (`-D warnings`) and `cargo test --workspace` exit 0
  (55 passed, 0 failed).
- Reflect done (owner, 2026-11-23). Open: the commit (DEBT-001), the first CI run (DEBT-005), the owner's
  look at the UI (DEBT-006).

## validate-origins: retrieved (DEBT-004, 2026-11-24)

The `modern-web-guidance` guide `validate-origins` was retrieved (npx is allowlisted again). Cross-check
against `hub/lab.js` `wsUrl()`:

```js
const wsUrl = () => { const u = new URL("/ws", location.href); u.protocol = u.protocol === "https:" ? "wss:" : "ws:"; return u.href; };
```

The function derives the socket URL from `location` itself, so no candidate string is compared and none of
the guide's spoofing classes apply: there is no prefix/substring match to bypass
(`https://app.example.com.attacker.example`), no unnormalized literal to equal, no public-suffix split, and
no opaque-origin comparison. The one remote-origin check on the page is the iframe guard
`if (e.origin !== location.origin || !e.data) return;`, an exact serialized-origin equality on a
same-origin `<iframe>` with no sandbox (`"null"` cannot appear), which is the case the guide keeps string
equality for. No change required; the guide is applied as a fact, not a fix.
