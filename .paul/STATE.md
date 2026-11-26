---
description: "vtradingview: current position and accumulated context"
type: ProjectState
about: "vtradingview"
---

# Project State

## Project Reference

See: .paul/PROJECT.md (updated 2026-11-23)

**Core value:** Free multi-feed TradingView-style charts in a browser or desktop app, with each user's
layouts and scripts kept in their Google account.
**Current focus:** Milestone v0.3 Bridge platform (created 2026-11-26): the MT5 bridge rewrites from
Python to Rust, distributed via mise; a Tauri manager app on the MT5 VPS installs, pairs (OTP from
Settings), controls and monitors it. Deployment stays terminal-local MCP; no cloud gateway
(RustMT5 rejected). v0.2 shipped complete; the site promotion and Q2/Q3/Q9 wait for the milestone
after v0.3.

## Current Position

Milestone: v0.3 Bridge platform - 1 of 2 phases
Phase: 09 of 2 (Bridge manager app) — 1 of 3 plans unified
Plan: 09-01 UNIFIED (hub pairing surface); 09-02 next
Status: 09-01 UNIFIED. All ACs green, five recorded deviations (AC-3/AC-5 Origin contradiction
resolved by a two-path narrow exemption; revoke secret as a side table rather than an unproven
ALTER; u64 rejection sampling; D-034 sequential execution). /security-review found and fixed two
defects: pairing codes were never swept (S14 clears sessions, whose TTL is 4,320x the code's) and
the new FK lacked an index.
Last activity: 2026-11-26T22:45+07:00 - 09-01 apply + unify in one session: migration 0004 +
P4 sweep, pairing routes, Settings dialog, 10 contract tests, four mutation proofs. DEBT-044
discharged, DEBT-043 dropped by the owner, DEBT-045 filed (check-mt5c.sh live-clock assertion).

- v0.1 Hosted launch: complete 2026-11-25 (4 phases, 13 plans)
- v0.2 Accounts everywhere: complete 2026-11-26 (3 phases, 12 plans)
- v0.3 Bridge platform: [████░░░░░░] 40% (phase 08 complete; 09-01 of phase 09 unified)

## Loop Position

Current loop state:
```
PLAN ──▶ APPLY ──▶ UNIFY
  ✓        ✓        ✓     [09-01 unified; 09-02 next]
```

## Accumulated Context

### Decisions
| D-037 | The two pairing paths (`/api/me/mt5/pair`, `/api/me/mt5/paired/{id}/revoke`) authenticate by a body secret and are exempt from the gate's Origin rule, matched structurally in `gate.rs::is_secret_authenticated`; the session-gated mint keeps the rule | Phase 09 | D-010 stops a cookie-riding page, and these paths never read the cookie; the Rust manager app sends no Origin. AC-3 vs AC-5 collided and this is the resolution. Written at 09-01 UNIFY. |
| D-034 | Plans carry `parallel_agents: false` here: the harness has no worktree isolation, so an APPLY wave runs task-by-task in plan order | Phase 07 | reflect outcome at 07-04; a parallel schedule without isolated copies races on the shared tree |
| D-035 | `py_str(f64)` reproduces Python `str(float)` and every wire string goes through it; Rust's `{}`/`{:e}` differ (`1829` vs `1829.0`, `1e15` vs `1000000000000000.0`) so the `/wait` `key` would diff red byte-for-byte | Phase 08 | measured against 18 Python outputs; the plan's Task 3 note claiming `{}` was equivalent is wrong (08-01 deviation 1) |
| D-036 | `term.rs` functions take `&M: Terminal` (a trait `Mcp` implements), so domain tests inject in-memory sources and no socket enters a domain test (L-016) while production passes the sealed `Mcp` | Phase 08 | the plan's `Terminals{live,hist,misc}` shape is unchanged; only the bound generalizes |
| D-029 | The deep link reaches the hosted page by a Rust-side `eval` of a `CustomEvent`, so the remote origin gets ONE permission, not the spec's two | Phase 06 | `core:event:default` is four permissions plus a cross-webview bus to an untrusted page; a fragment navigation reloads and kills the in-memory `desktop_key`. Written at 06-04 APPLY. |
| D-030 | `crates/desktop` is its own Cargo workspace, not a root member; it gets its own CI job | Phase 06 | a second workspace cannot path-depend on a crate using `edition.workspace = true`; the root `cargo test --workspace` and gnu host pin must stay untouched. Written at 06-04 APPLY. |
| D-031 | The 256-key cap stays corrective; this is the decision DEBT-033 reserved | Phase 06 | bounded by the per-IP bucket and single-user scope; preventive form needs a subquery D-014 disallows. Written at 06-04 APPLY. |
| D-032 | The bridge surface contract (R24): ws subscribe gains an optional `bridge` field (source=mt5 without it keeps the Binance fallback); bridge symbols cached at every successful Test and merged into /api/symbols for the session owner only; one poller per (bridge, symbol, tf) long-polling /wait; failures surface the exact cause verbatim; cross-user is 404; no new rate bucket | Phase 07 | the anonymous surfaces and D-009 parity hold by construction; the semaphore plus shared poller is the spec's per-user bound. Approved at the phase-07 plan approval 2026-11-26. |
| D-033 | Local mode carries one env-configured bridge: VTV_LOCAL_BRIDGE_URL + VTV_LOCAL_BRIDGE_TOKEN exposed as bridge id `local`; the SSRF guard allows loopback only when auth is off (D-001's exception gets its mechanics) | Phase 07 | local mode has no database or users, so a registered bridge cannot exist there; the env bridge is also the served-test seam. Approved at the phase-07 plan approval 2026-11-26. |
| 2026-11-25: A signed-out caller gets 401 before any key check on all four `/api/me/state` routes (security-review ordering fix; AC-6) | Phase 05 | 05-03's client can read 401 as "sign in again", never a disguised 404/204 |
| D-019 | The header control is sticky to the bar's right edge (the bar scrolls sideways at 1280 px) | Phase 03 | #authSlot |
| D-020 | Delete confirmation uses native `<dialog>` + showModal(); the browser owns the focus trap and Esc (modern-web-guidance "dialog" Rule 1) | Phase 03 | auth.js |
| D-021 | `VTV_TEST_IDP=1` makes the test-idp build serve `/__test-idp/*`; release builds can never reach it (feature-gated, CI asserts the tree) | Phase 03 | lib.rs |
| D-022 | The TTFC beacon posts with fetch keepalive only (sendBeacon sends text/plain, which post_guard 415s); ms measured with performance.now() inside ttfc.js, so pages need no t0 | Phase 04 | hub/ttfc.js |
| D-023 | DEBT-015 discharged by wiring: check-0303 runs inside check-0402.sh (CHECK_0303=1) against the harness's own test-idp server; the CI-headless half narrowed into DEBT-017 | Phase 04 | check-0402.sh |
| D-018 | Pools check a connection (SELECT 1) before handing it out; pgsqlite desyncs after an error | Phase 03 | db::pool_options |
| D-017 | /auth/me carries the public client_id | Phase 03 | auth.js |
| D-016 | No DB readiness on /api/health in Phase 3 | Phase 03 | health unchanged |
| D-015 | Any desktop exchange attempt burns the code | Phase 03 | one-time code |
| D-014 | Single-table SQL only (pgsqlite via sqlx); unique conflict = 23505 or 42000 message | Phase 03 | store shape |
| D-013 | Phase 3 ships sign-in UI and avatar menu; sync.js from Phase 5 | Phase 03 | no 404 for sync.js |
| D-012 | Logs public in local mode, admin-only with auth on; panel polls only its route | Phase 03 | /admin/logs |
| D-011 | Own migration runner, plain DDL, ids and times from Rust; PGSQLITE_BIN default ~/.local/bin/pgsqlite | Phase 03 | no sqlx migrator |
| D-010 | CSRF by Origin + SameSite=Lax, no token (N5) | Phase 03 | C1 closed |
| D-009 | `feed=mt5|arm` -> Feed::Off (502 only where history is called); Python quirks ported before the parity gate | Phase 02 | real delta still 200 |
| D-008 | Order-flow cache: Rust-only column format under `hub/.cache/orderflow/`, not parquet | Phase 02 | adds `zip` crate; no arrow/parquet |
| D-007 | Rate limit: in-crate per-IP token bucket (20/s, burst 40), not tower_governor | Phase 01 | `/api/*` 429 + Retry-After |
| D-006 | Tracing target `server`/`feed`, provider in field `src`; RingLayer builds `feed.<src>` | Phase 01 | 01-03 RingLayer mapping |
| D-005 | Pin axum-0.8-compatible crate versions; Cargo.lock is the pin | Phase 01 | newer majors only in a plan that passes the gates |
| D-004 | Closed bar published once on roll-over, bypassing the throttle | Phase 01 | fixes lost closing OHLC; one extra frame per bar close |
| D-003 | Keep `mt5`/`arm` as "off" in /api/symbols (amends D-001) | Phase 01 | lab.js unchanged except wsUrl() |
| D-002 | Keep today's websocket protocol (one subscription per socket); 24 sockets per IP; upstream shared per (source, symbol) | Phase 01 | lab.js changes only the URL; /ws parity testable against Python |
| D-001 | Local mode for charts only (auth off, no DB, 127.0.0.1); MT5 only through Phase 07 bridges, loopback allowed when auth is off | Phase 01 | removes Local MT5 and arm until Phase 07; tag `python-final` |
- Session rules R-7..R-10 (probe runtime Facts, bare exit status, complete gate set, re-run mutations after
  editing a checker) came out of the 03-03 phase reflect; full text in `.paul/SPECIAL-FLOWS.md`.
- Imported from `.paul/PLANNING.md` and the design doc; see PROJECT.md Key Decisions.
- Owner concluded every EGM review stage with findings open; they are carried as phase gates in
  ROADMAP.md, not applied to the spec.

### Deferred
- DEBT-001 done 2026-11-23 (commit 7f0b46f).
- DEBT-007 done 2026-11-23 (commit 6ae3ea1).
- DEBT-011 open (blocking phase-03-exit): dev db binds 0.0.0.0; needs pgsqlite with --host (see .paul/DEBT.toml).
- DEBT-012 open: pgsqlite 0.0.22 mis-maps UPDATE parameters out of text order; S4/S8 work around it.
- DEBT-004 done 2026-11-24 (validate-origins retrieved; wsUrl() needs no change).
- DEBT-009 done 2026-11-24 (menu, dialog, secure-external-subresources retrieved; applied in 03-03).
- DEBT-011 done 2026-11-24 (pgsqlite fork built and installed: `--host 127.0.0.1`; old binary kept as
  `~/.local/bin/pgsqlite.0.0.22.bak`; the dev db no longer binds 0.0.0.0).
- DEBT-014 open (owner, phase:04): `semantic.text.link` token missing, `validate_contrast` fails both themes (pre-existing).
- DEBT-003 done 2026-11-25 (hub/lab.js reconnect with backoff shipped; observed in check-0402's kill-restart).
- DEBT-015 done 2026-11-25 (check-0303 wired into check-0402.sh, `CHECK_0303=1`; 66/66 under the shipped CSP; the CI-headless half lives in DEBT-017).
- DEBT-016 open (agent, phase:04): render gates and the huashu scripts load `file://` only, so a JS-injected UI measures empty.
- DEBT-017 open (agent, phase:04): the five-gate design set is run piecemeal; a single runner that diffs the receipt set is needed.
- DEBT-033 open (agent, phase:06): the 256-key cap on `/api/me/state` is corrective, not
  preventive; revisit with Q7 under D-026.
- DEBT-034 open (agent, phase:06): the `/api/me/state` contract lives only in 05-02's plan table
  and SUMMARY; `docs/` has no page.
- DEBT-013 done 2026-11-25 (the https callback registered in the console; proven by the live 302
  probe recorded in 06-02's SUMMARY).
- DEBT-033 done 2026-11-25 (check re-pointed at D-031 and discharged; the old check was vacuous —
  it read a pre-existing D-026 and passed on day one).
- DEBT-034 done 2026-11-25 (docs/api.md written from state.rs's module doc).
- DEBT-036 open (owner, phase:06): Q4 code signing — a Windows EV/OV certificate and an Apple
  Developer ID are missing; installers stay unsigned beta until wired (secrets-only change).
- DEBT-037 done 2026-11-25 (the 06-04 closing checkpoint was approved — owner looked at the window).
- DEBT-038 done 2026-11-26 (desktop.yml run 45767331583 green: check-windows, check-linux, root
  test job tauri/webkit-free).
- DEBT-039 done 2026-11-26 (owner saw live candles through their tunnel; the public half verified
  from the dev box).
- DEBT-043 dropped 2026-11-26 by the owner: a Cloudflare quick tunnel's temporary domain is
  acceptable, so a named tunnel is not owed. The AT-STARTUP hardening lives in 09-02/09-03's
  install_stack, not in that row.
- DEBT-044 done 2026-11-26 (09-01 task 5: the five parity-suite files force-added; git ls-files = 5;
  the differ was already un-sorted at the phase-08 reflect).
- DEBT-046 open (agent, phase:09): DECISIONS.md lacks D-035 and D-036 — phase 08 wrote both to the
  STATE table only, so the id list jumps D-034 to D-037. Same class as DEBT-027 (discharged in phase
  05), so the transcription step regressed. Filed at 09-01 UNIFY.
- PILE-UP: 16 non-discharged rows at 09-01 close, against the budget of 12. Per the UNIFY rule the
  next PLAN must schedule at least one discharge; DEBT-046 and DEBT-045 are the two cheapest (both
  are transcription/check fixes, no design work).
- DEBT-045 open (agent, phase:10): check-mt5c.sh's logged-out /health assertion canonically diffs a
  live `measured_at` across two sequential curls, so the suite exits 1 whenever they straddle a
  second boundary (33/33 parity still passes both ways). Filed at 09-01; R-10: fixing it invalidates
  the phase-08 mutation results, so re-run mutate-mt5c.sh afterwards.

### Skill Audit (Phase 03, plan 02)
| Expected | Invoked | Notes |
|----------|---------|-------|
| /architect | yes | plan stage: 03-ARCHITECTURE.md |
| /security-review (Phase 03 override) | yes | whole plan diff reviewed; dispositions in the SUMMARY |
| /paul-security | n/a | optional; a report at phase exit, not per plan |
| /reflect (pstack) | deferred | runs at the UNIFY of 03-03 (phase's last plan), not here |
| context-budget, caveman, ponytail | yes | loaded and applied this session (AGENTS.md rule) |

### Skill Audit (Phase 03, plan 03)
| Expected | Invoked | Notes |
|----------|---------|-------|
| /design-doctrine, /design-component, /design-code | yes | kit bodies read; menu and dialog specced from the mockups before code |
| /design-taste-frontend, /huashu-design | partial | harness built and gated; `polish.py shot` cannot load a served page (DEBT-016) |
| /a11y-audit | yes | axe 0 violations both themes, verify_states 36/36 AA, 2.4.11 occlusion assertion added |
| /design-gate (unify) | yes | run at UNIFY; the incomplete receipt set is DEBT-017 |
| /modern-web-guidance | yes | menu, dialog, secure-external-subresources, validate-origins retrieved and applied |
| /security-review (Phase 03 override) | yes | at APPLY; the UNIFY reflect added the release-guard fix |
| /reflect (pstack) | yes | phase scope, 3 reviewers + synthesizer, R-7..R-10, L-019..L-023 |
| /rsi | yes | no candidate; DEBT-016/017 carry the two that need building |

### Git State
Last commit: 7317378 feat(08): rust MT5 bridge, mise distribution, python bridge retired (30 files)
Branch: arch/axum-tauri (pushed through 914740b; the phase-08 commits are not pushed yet; the
next push carries them; release.yml's new legs run on the next tag)
Uncommitted: none (the phase commits landed everything; the suite's oracle copy stays in
gitignored .paul/runs/)

### Blockers/Concerns
- Design doc Blockers still in the spec: C1, C2, R1 to R7 (see ROADMAP.md gates and
  `.paul/design/reviews/vtradingview/implementation_readiness_report.md`).
- Public launch blocked on Q2 (data terms), Q3 (trademark), Q9 (privacy text).

## Session Continuity

desktop.yml run observed green (check-windows, check-linux; root test job tauri/webkit-free) — run
45767331583 on c548cdc, 2026-11-26: check-windows 5m14s success (cargo test + clippy on
crates/desktop), check-linux 10m46s success, installers skipped (secrets absent, by design). Same
push's ci.yml run 45767331529: all seven jobs success (fmt, clippy, test — cargo test --workspace
tauri/webkit-free, audit, db, contract, browser-gates). DEBT-038 discharged.

**DEBT-039 discharged: real bridge streams candles observed** (owner, 2026-11-26). The owner ran
bridge/mt5_bridge.py on the MT5 host (103.93.92.204, the Exness terminal "Exness-MT5Real28",
Python 3.15.2 + cloudflared via mise, published through a Cloudflare quick tunnel), registered it
in Settings, and watched live candles stream on the broker's XAUUSDm chart. The agent verified the
public half beforehand (all http 200 through the tunnel from the dev box): /health ok, /bars
XAUUSDm M1 returning live candles, /wait long-poll fresh bar, /symbols 358. The second-account
negative half (another login sees neither the bridge nor its symbols) is enforced by construction
(owner-scoped routes, cross-user 404) and proven by the 07-03 suites. The quick-tunnel durability
gap was filed as DEBT-043 and **dropped by the owner on 2026-11-26**: a quick tunnel's temporary
domain is acceptable, so no named tunnel is owed.

Last session: 2026-11-26T19:05+07:00 (phase 08 committed and transitioned)
Stopped at: Phase 08 complete (7317378); phase 09 Bridge manager app next.
Next action: phase reflect (pstack) at the session reset, then /paul:plan for phase 09.
Resume file: none (the phase-08 handoff was consumed and archived at .paul/handoffs/archive/HANDOFF-2026-11-26-phase08.md)
Resume context:
- Milestone v0.3 Bridge platform (phases 08/09) created 2026-11-26; ROADMAP and dirs exist.
- Phase 08 contract: .paul/phases/08-rust-bridge/08-ARCHITECTURE.md (5-module crate, 7-rule
  parity contract, fake-MCP-terminal test architecture, windows-first distribution).
- 08-01 shipped `Terminal` (trait) + `Mcp` (sealed leaf) + `term` math; 08-02 adds live.rs,
  http.rs and the real main.rs, and brings the `subtle` + `dotenvy` deps it needs.
- RustMT5 and mcp-proxy both verified and rejected (cloud gateway / Tower aggregator).
- D-032 frozen surface; D-035 py_str; D-036 Terminal seam; D-005 pins; workspace stays
  tauri-free; L-003/L-009/L-016/L-026/R-8 govern every check; the real MT5 host
  (103.93.92.204, Windows) is phase 09's checkpoint.
- Debt: 0 overdue (all checks OK 2026-11-26), 15 waiting; DEBT-040/041/042 -> phase:10;
  DEBT-043 event fired (phase 09).

**Phase-08 reflect ran in this session** (pstack panel, 2026-11-26T19:05+07:00; consumed
HANDOFF-2026-11-26-phase09.md and session-reset.json). 3 lenses, parent-verified by live probe.
One live defect found and fixed: /symbols key order (preserve_order was dev-only; the built
binary shipped BTreeMap order against contract rule 6; differ's sort_keys hid it). Differ now
un-sorts keys and pins total == 33. All gates re-run green after the edit (R-10): fmt, clippy,
40 crate tests, 200 workspace tests, 33/33 parity both passes, 6 mutations red/green.
Filed: L-051..L-054, L-023 hit 6, L-026 hit 3, DEBT-044 (parity suite untracked; trigger
phase:09). Pending owner yes: R-8 tighten + context-budget exemption line (see below).

Last session: 2026-11-26T23:05+07:00 (/paul:pause, owner-requested)
Stopped at: 09-01 UNIFIED (apply + unify + /security-review in one session). All gates green:
fmt 0, clippy plain 0, clippy test-idp 0, workspace 0 (tauri-free), hosted 0 (19 binaries),
pairing_routes 10 passed, gates 8/8 x 5 pages, receipts 0, debt 0 overdue. Committed as 97d57c3
(83 files), so UNIFIED = landed.
Next action: /paul:apply
.paul/phases/09-bridge-manager/09-02-PLAN.md (the manager Tauri app; its blocker is the 09-01
contract, now settled: pair takes {code, label, url, token} with no cookie and no Origin, 201
carries revoke_code exactly once, and a refused attempt burns the code).
Resume file: .paul/HANDOFF-2026-11-26-phase09-01.md
Resume context:
- The five deviations and the two security findings are in 09-01-SUMMARY.md; do not re-derive.
- D-037 pins the Origin exemption to exactly two paths; the mint keeps the rule.
- PILE-UP: 16 non-discharged rows vs a budget of 12 - the next plan must schedule one discharge
  (DEBT-045 and DEBT-046 are the cheap ones).
- check-mt5c.sh exits 1 on a pre-existing live-clock assertion (DEBT-045), not a 09-01 regression;
  its differ still reports 33/33 twice.
- Owner decision stands: do NOT deploy; the live site stays v0.1.1.
- Two instruction edits still await the owner's yes: R-8 tighten, context-budget judged-gate line.
Resume context:
- Reflect outcomes at 07-04: L-024/L-029 validated, L-027 generalized, L-048..L-050 added,
  D-034 (parallel_agents false), R-12, DEBT-040/041 filed; DEBT-042 carries the phase-07
  security dispositions.
- Instruction edits awaiting the owner's yes: tune design-taste-frontend's look-pass step
  ("Read the shots; say explicitly when the harness cannot render images"); R-8 tighten
  (bare `; echo exit=$?` only valid with nothing piped); context-budget exemption line
  (judged gate commands are exempt from the no-single-call rule).
- Owner decision stands: do NOT deploy; the live site stays v0.1.1.
- CARL GLOBAL rules: justrust for cargo, ygrep/agentgrep first for searches.
