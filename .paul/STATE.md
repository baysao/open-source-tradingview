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
**Current focus:** Phase 03: Database and Google sign-in (03-02 APPLY in flight)

## Current Position

Milestone: v0.1 Hosted launch
Phase: 03 of 07 (Database and Google sign-in), in progress
Plan: 03-02 APPLIED (3 of 3 tasks; commit 3cf8721); ready for UNIFY
Status: APPLY complete - suites green on pgsqlite, 22/22 mutations fail, gates exit 0
Last activity: 2026-11-24 14:44 (+07) - 03-02 APPLY complete (SUMMARY written)

Progress:
- Phase 01: complete 2026-11-23 (milestone v0.1: 1 of 4 phases)
- Phase 02: complete 2026-11-23 (reflect deferred: owner has not run /reflect)
- Phase 03: [████░░░░░░] 1 of 3 plans unified (03-01), 03-02 applied (awaiting UNIFY)

## Loop Position

Current loop state:
```
PLAN ──▶ APPLY ──▶ UNIFY
 done     done     ○     [03-02 APPLIED - next is UNIFY 03-02]
```

## Accumulated Context

### Decisions
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
- Imported from `.paul/PLANNING.md` and the design doc; see PROJECT.md Key Decisions.
- Owner concluded every EGM review stage with findings open; they are carried as phase gates in
  ROADMAP.md, not applied to the spec.

### Deferred
- DEBT-001 done 2026-11-23 (commit 7f0b46f).
- DEBT-007 done 2026-11-23 (commit 6ae3ea1).
- DEBT-011 open (blocking phase-03-exit): dev db binds 0.0.0.0; needs pgsqlite with --host (see .paul/DEBT.toml).
- DEBT-012 open: pgsqlite 0.0.22 mis-maps UPDATE parameters out of text order; S4/S8 work around it.

### Blockers/Concerns
- Design doc Blockers still in the spec: C1, C2, R1 to R7 (see ROADMAP.md gates and
  `.paul/design/reviews/vtradingview/implementation_readiness_report.md`).
- Public launch blocked on Q2 (data terms), Q3 (trademark), Q9 (privacy text).

## Session Continuity

Last session: 2026-11-24 14:44 (+07) (03-02 APPLY complete)
Stopped at: 03-02 APPLIED - auth 14, admin 1, gate 4 green on pgsqlite; 22/22 mutations fail; fmt and clippy exit 0; SUMMARY at .paul/phases/03-auth/03-02-SUMMARY.md
DUE (2026-11-23, deferred by owner): /reflect for Phase 02 (reflect: pstack)
~~DUE NOW (2026-11-23): 01-03 checkpoint:human-verify, owner checks the hub on hub-server (DEBT-002)~~ closed 2026-11-23: verified by agent with Playwright at the owner's request
~~DUE NOW (2026-11-23): owner types /reflect for the Phase 01 close (reflect: pstack)~~ done 2026-11-23 (6 accepted edits applied, R-6, L-010..L-012)
Next action: /paul:unify .paul/phases/03-auth/03-02-PLAN.md; before 03-03 APPLY, allowlist `npx -y modern-web-guidance@latest *` (DEBT-009); DEBT-011 (pgsqlite --host) blocks the Phase 03 exit; DEBT-010 is the owner's real-Google look
Resume file: .paul/phases/03-auth/03-02-SUMMARY.md

---
*STATE.md: updated after every significant action*
