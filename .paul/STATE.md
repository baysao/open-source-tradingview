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
**Current focus:** Milestone v0.1 shipped its four phases; the public launch still waits on owner rows
(DEBT-018 host, DEBT-013 OAuth URI, Q2/Q3/Q9, DEBT-005 CI). Milestone v0.2: phase 05 complete and
unified 2026-11-25; phase 06 (Tauri desktop) is next to plan.

## Current Position

Milestone: v0.2 Accounts everywhere
Phase: 06 of 07 (Tauri desktop) - Not started
Plan: Not started
Status: Ready to plan
Last activity: 2026-11-25 - Phase 05 complete and unified (4/4 plans). Close-out: reflect panel 20
accepted (3 code fixes A15-A17 with harness cases, D-026/D-027/D-028, L-037 validated by experiment
.paul/experiments/L-037.md, 5 new candidate rows, L-003/L-007/L-029 bumps); rsi run. Final
verification: check-0503 22/22, harness 19/19, gate_all 8/8 + receipts, check-0402 56/56,
CHECK_0303=1 → 66/66, workspace 139 passed, fmt+clippy clean, phase-05 debt gate clear.

Progress:
- Phase 01: complete 2026-11-23 (milestone v0.1: 1 of 4 phases)
- Phase 02: complete 2026-11-23 (reflect deferred: owner has not run /reflect)
- Phase 03: complete 2026-11-24 (3 of 3 plans unified; reflect + RSI done at 03-03)
- Phase 04: complete 2026-11-25 (3 of 3 plans unified; phase reflect run)
- Phase 05: complete 2026-11-25 (4 of 4 plans unified; phase reflect + rsi run)

## Loop Position

Current loop state:
```
PLAN ──▶ APPLY ──▶ UNIFY
  ○        ○        ○     [Phase 05 complete; phase 06 not started]
```

## Accumulated Context

### Decisions
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
- DEBT-013 open (owner, phase:04): register the loopback Google redirect URI and serve hosted mode over HTTPS; the prerequisite for DEBT-010.

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
Last commit: d235b03 feat(05): synced state (the phase commit; 48 files)
Branch: arch/axum-tauri (not pushed; DEBT-005 was discharged 2026-11-25 by the first green CI run)
Uncommitted: check the tree

### Blockers/Concerns
- Design doc Blockers still in the spec: C1, C2, R1 to R7 (see ROADMAP.md gates and
  `.paul/design/reviews/vtradingview/implementation_readiness_report.md`).
- Public launch blocked on Q2 (data terms), Q3 (trademark), Q9 (privacy text).

## Session Continuity

Last session: 2026-11-25 (phase 05 UNIFY + transition)
Stopped at: Phase 05 complete, ready to plan Phase 06
Next action: /paul:plan for Phase 06 (Tauri desktop)
Resume file: .paul/ROADMAP.md
Session rule: after each plan's APPLY, /paul:pause, owner clears, next plan via /paul:resume in a
new session (CARL GLOBAL rule 2; config session_reset: plan).
Resume context:
- Phase 05's surface is uncommitted until the phase commit (which the transition performs next):
  crates state routes + migrations, hub/sync.js + page wiring, docs/vela-storage-keys.md, the two
  test drivers, all .paul state files.
- Reflect panel close-out applied: 3 code fixes (A15 unbacked-merge adoption, A16 per-key conflict
  notices, A17 raced keep-mine) each with a harness case; D-026/D-027/D-028 recorded; L-037
  validated by .paul/experiments/L-037.md; L-003→8, L-007→2, L-029→2; new candidates L-038..L-042.
- The harness count is L-026-enforced: sync_harness.js CASES_EXPECTED=19, asserted in the run body.
- Both test servers for 05-03 were throwaway (pgsqlite /tmp scratch on 55495, test-idp hub on 8771,
  local on 8770); nothing in the repo points at them.
- Playwright authenticated reads go through in-page fetch (L-037), never ctx.request (Secure cookie).
- test-IdP accounts key on sub, not email; shared-dev-DB drivers scope entities per run (L-038).
- CARL GLOBAL rules: justrust for cargo, ygrep/agentgrep first for searches, obscura first for
  browser checks.
