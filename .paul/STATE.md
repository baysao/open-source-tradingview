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
**Current focus:** Project initialized; ready to plan Phase 01

## Current Position

Milestone: v0.1 Hosted launch
Phase: 02 of 07 (Order flow in Rust), applying
Plan: 02-01, 02-02 and 02-03 UNIFIED
Status: Phase 02 plans closed; phase close waits for /reflect (owner) and the commit
Last activity: 2026-11-23, project initialized from SEED launch

Progress:
- Phase 01: complete 2026-11-23 (milestone v0.1: 1 of 4 phases)
- Phase 02: [██████████] 3 of 3 plans closed

## Loop Position

Current loop state:
```
PLAN ──▶ APPLY ──▶ UNIFY
  ●        ●        ●     [02-03 closed; phase reflect next]
```

## Accumulated Context

### Decisions
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
- DEBT-007 (open, artifact): commit the Phase 02 work; checked against HEAD by `debt.py`.

### Blockers/Concerns
- Design doc Blockers still in the spec: C1, C2, R1 to R7 (see ROADMAP.md gates and
  `.paul/design/reviews/vtradingview/implementation_readiness_report.md`).
- Public launch blocked on Q2 (data terms), Q3 (trademark), Q9 (privacy text).

## Session Continuity

Last session: 2026-11-23
Stopped at: 02-03 UNIFIED (uncommitted: DEBT-007)
DUE NOW (2026-11-23): owner types /reflect for the Phase 02 close (reflect: pstack)
~~DUE NOW (2026-11-23): 01-03 checkpoint:human-verify, owner checks the hub on hub-server (DEBT-002)~~ closed 2026-11-23: verified by agent with Playwright at the owner's request
~~DUE NOW (2026-11-23): owner types /reflect for the Phase 01 close (reflect: pstack)~~ done 2026-11-23 (6 accepted edits applied, R-6, L-010..L-012)
Next action: /reflect (owner, Phase 02 scope), then commit Phase 02 when the owner asks (DEBT-007)
Resume file: .paul/phases/02-orderflow/02-01-PLAN.md

---
*STATE.md: updated after every significant action*
