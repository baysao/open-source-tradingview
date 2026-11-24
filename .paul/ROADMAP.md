---
description: "vtradingview: milestone and phase structure"
type: Roadmap
about: "vtradingview"
---

# Roadmap: vtradingview

## Overview

Move TradeX from a local Python hub to a hosted, multi-user service at `https://vtradingview.com`, with a
Rust (Axum) server, Google accounts that sync each user's work, and a Tauri 2 desktop shell. Brief:
`.paul/design/specs/vtradingview.md`.

## Current Milestone

**v0.1 Hosted launch** (v0.1.0)
Status: In progress
Phases: 3 of 4 complete

## Milestones

| Milestone | Phases | Goal |
|-----------|--------|------|
| v0.1 Hosted launch | 01 to 04 | the public site with live charts and Google sign-in |
| v0.2 Accounts everywhere | 05 to 07 | synced state, desktop app, per-user MT5 |

## Phases

| Phase | Name | Milestone | Plans | Status | Completed |
|-------|------|-----------|-------|--------|-----------|
| 01 | Rust workspace and hub-server parity | v0.1 | 3 (01-01, 01-02, 01-03) | Complete | 2026-11-23 |
| 02 | Order flow in Rust | v0.1 | 3 (02-01, 02-02, 02-03) | Complete (reflect deferred) | 2026-11-23 |
| 03 | Database and Google sign-in | v0.1 | 3 (03-01, 03-02, 03-03) | Complete | 2026-11-24 |
| 04 | Public deploy | v0.1 | TBD | Not started | - |
| 05 | Synced state | v0.2 | TBD | Not started | - |
| 06 | Tauri desktop | v0.2 | TBD | Not started | - |
| 07 | Per-user MT5 bridges | v0.2 | TBD | Not started | - |

## Phase Details

Every phase plan first applies the design-doc fixes in its **Gate** row. The findings come from the EGM
review in `.paul/design/reviews/vtradingview/` (owner concluded each stage with them open, 2026-11-23);
the proposed but unapplied fixes are in `goldfish_review_report.md`. A gate is cleared when the spec says
the right thing, not when the code does.

### Phase 01: Rust workspace and hub-server parity
- **Build:** Cargo workspace (`hub-types`, `hub-core`, `hub-server`), static allow-list, existing `/api/*`
  routes except order flow, `/ws` on port 8770, Deriv, Bybit and Binance feeds through FeedHub, log bus,
  `Procfile` + `scripts/dev.sh` (`hub` line), CI.
- **Accept:** a contract suite that diffs raw JSON (key sets and types, websocket `type` values and field
  sets) passes against `python hub/server.py` and `hub-server`; `/.env` is 404; one port only.
- **Gate: cleared 2026-11-23** (D-001, D-002, spec Phase 1 facts rewritten against the code). Was: E1 self-hosting decision, and with it the `mt5`, `arm`, `ws_port` fields
  (R14); websocket protocol kept as today's one subscription per socket or redesigned (R1, N1); per-IP
  socket limit that fits an 8-chart layout (R2); facts about today's server corrected: log ring 12,000
  (R7), logbus and feedlog parity (R8), `/api/health` clock offset and lag (R9), `/` serves
  `hub/index.html` (R11), restart and journal routes (R12); contract oracle is raw JSON, not
  deserialisation (R31, N4); crate list (R17); CI live-upstream rule (R30); clippy command (R29).
- **Risks:** upstream bans of CI and data-centre IPs; protocol drift breaking the frontend.

### Phase 02: Order flow in Rust
- **Build:** delta, footprint and session volume profile ports, Binance USD-M futures trade archives and
  REST trades, the candle-estimate fallback, golden fixtures.
- **Accept:** every fixture matches within the stated relative tolerance; numpy, pandas and pyarrow are no
  longer needed by the hub.
- **Gate: cleared 2026-11-23** (spec Phase 2 facts and items 16 to 18 rewritten against the code). Was: cache
  is parquet under `hub/.cache/binance/` and `hub/.cache/rest_trades/` (R5): new format and path or real
  parquet; `tpo.rs` removed or specified, TPO is browser-side (R6); weight budget and constants (R10);
  deterministic fixture capture, `XAUUSDT` mapping (R32).

### Phase 03: Database and Google sign-in
- **Build:** migrations (users, identities, sessions, desktop codes), pgsqlite `db` line, One Tap and
  redirect sign-in, sessions, admin gate (`/admin/*`), account deletion.
- **Accept:** first Google sign-in creates exactly one user, the second reuses it; CI green on pgsqlite
  and Postgres 17; non-admins get 404 on every `/admin/*`.
- **Gate: cleared 2026-11-23** (spec "Phase 3 facts and decisions"; D-010 to D-013). Was: CSRF approach (C1: derived token, or drop the token per N5 with `Origin` required); full
  column tables with types, indexes and cascades (R3); request and response shapes for every new route
  (R4); session slide, cookie MAC, key separation, issuer rule (R21); `AUTH_ENABLED` behaviour (R18);
  log panel polling gated on admin status (R13); `sync.js` not loaded before Phase 05 (R27).
- **Plans (2026-11-24):** 03-01 config, db, migrations; 03-02 auth and routes; 03-03 frontend sign-in,
  log panel gating, browser check. Contract: `phases/03-auth/03-ARCHITECTURE.md` (D-014 to D-017).

**Phase 03 exit (2026-11-24):** all three plans unified. 03-01 config, schema and the migration runner;
03-02 the auth module, gate and route tables; 03-03 the header control, avatar menu, delete dialog and the
gated log panel. Gates: `cargo test --workspace` 118 passed, fmt and clippy clean in both feature ways,
the browser check 66/66 with four mutations each red then restored, axe 0 violations both themes. The
dev db now binds loopback (`pgsqlite --host`, DEBT-011 discharged). Still open: the first green CI run
(DEBT-005), the real-Google round trip (DEBT-010, blocked on DEBT-013), and the owner's UI look.

### Phase 04: Public deploy
- **Build:** production host, Cloudflare Tunnel `tunnel` line, rate limits, CSP and security headers,
  backups and restore drill, licence and AGPL source notices, privacy page, TTFC beacon.
- **Accept:** live charts through the tunnel; the host has no inbound ports; restore drill done; AGPL
  source link live; TTFC visible on the admin panel.
- **Gate:** Risks & Mitigations section in the spec (C2); CSP string that keeps the inline scripts,
  Google Fonts, Vela Workers and GIS working, and rate-limit values (R19); feed cap counts symbols and
  reserves the default chart (C14, N2); TTFC defined once and persisted (C16); backup destination (R26).
  Launch blocked on Q2, Q3, Q9.

### Phase 05: Synced state
- **Build:** `user_state`, `/api/me/state`, sign-in merge, `sync.js`, conflict and backup notices,
  header sign-in UI per the D4 mockups.
- **Accept:** a layout drawn signed-out appears after sign-in in a second browser; cross-user tests pass.
- **Gate:** Vela key inventory first (Q5); merge finishes before the iframe loads, merge writes excluded
  from upload (N3); per-tab versions (N7); `tradexDrawFavs` and `tradexFavBar` decision (R16); delete,
  key-limit, backup-collision and offline cases (R23); copy deck and states (C7, C13).

### Phase 06: Tauri desktop
- **Build:** `crates/desktop`, deep-link sign-in handoff, tray, capability lock-down, installers.
- **Accept:** the installed app signs in through the system browser and shows the same synced layout.
- **Gate:** how the page gets or avoids `desktop_key` (K4, N6, R25); webkit2gtk in CI (R27); signing (Q4).

### Phase 07: Per-user MT5 bridges
- **Build:** bridge token and HTTPS mode, `mt5_bridges`, SSRF guard, bridge routes, settings screen.
- **Accept:** a real bridge streams candles for its owner; every SSRF case is refused; no other user can
  see or use the bridge.
- **Gate:** how bridges reach the symbol picker and the subscribe format (R24); start only after Phase 04
  has live users (Z5).

---
*Roadmap created: 2026-11-23*
