---
description: "Free multi-feed TradingView-style charts in browser and desktop, with each user's layouts and scripts kept in their Google account"
type: Project
about: "vtradingview"
---

# vtradingview

## What This Is

A hosted, multi-user TradeX at `https://vtradingview.com/hub.html`. A Rust (Axum) server replaces the
Python hub (`hub/server.py`), Google sign-in saves each user's work to a Postgres-protocol database
(pgsqlite by default, real Postgres in production), and a Tauri 2 app shows the same site on the
desktop. Built on branch `arch/axum-tauri`.

Brief (single source): `.paul/design/specs/vtradingview.md`. Plan: `.paul/PLANNING.md`.

## Core Value

Free multi-feed TradingView-style charts in a browser or desktop app, with each user's layouts and
scripts kept in their Google account.

## Current State

| Attribute | Value |
|-----------|-------|
| Type | Application (complex) |
| Version | 0.0.0 |
| Status | Initializing |
| Last Updated | 2026-11-26 |

## Requirements

### Core Features

- Same charts, new server: every existing `/api/*` route and websocket message keeps its shape (G1)
- p75 time to first live candle under 2 s, measured in visitors' browsers (G2)
- Google sign-in with auto login for returning users and account creation on first sign-in (G3)
- Saved state follows the user across browsers and the desktop app; work done before sign-in is kept (G4)
- No data crosses between users (G5, hard guardrail)
- Per-user MT5 bridges, with every private, loopback or link-local target refused (G6)
- One database switch: `DATABASE_URL` from pgsqlite to Postgres needs no code change (G7)
- Closed origin: all traffic through the Cloudflare Tunnel (G8)

### Validated (Shipped)
- ✓ Same charts on the new server: `/api/*` and `/ws` keep their shapes, contract suite vs the Python hub (G1) - Phase 01
- ✓ Order flow in Rust: delta, footprint and session profile from fixtures within tolerance, no numpy/pandas/pyarrow (G2) - Phase 02
- ✓ Google sign-in: first sign-in creates one user, the second reuses it, sessions revocable, admin gate 404s outsiders (G3) - Phase 03

### Active (In Progress)
- [ ] Closed origin and public deploy through the Cloudflare Tunnel, with backups and a restore drill (G8) - Phase 04

### Validated (Shipped) — v0.2 additions
- ✓ Synced state per user: layouts and scripts identical across browsers, nothing overwritten
  silently (G4) - Phase 05 — a signed-out layout appears in a second browser after sign-in; a
  concurrent edit 409s with both values recoverable; clashing keys back up and Restore works
  (harness CUJ3/4/5, AC-8; check-0503 22/22, harness 19/19)
- ✓ Desktop shell: the installed app signs in through the system browser (the `vtradingview://`
  hand-off) and shows the same synced layout - Phase 06 — the remote origin holds ONE permission
  (`start_sign_in`), the nav guard refuses off-origin, tray Show/Quit; checkpoint approved
  2026-11-25 (owner looked at the window); 31 crate tests + 5 phase mutations red-then-green,
  gate receipts 8/8

### Planned (Next)
- v0.1 Hosted launch: Rust parity, order flow in Rust, database and Google sign-in, public deploy
- v0.2 Accounts everywhere: synced state, Tauri desktop, per-user MT5 bridges

### Out of Scope
- Order placement or any trading action (read-only market data)
- Rewriting the frontend (no Yew, React or Vite)
- A backend inside the desktop app, offline mode, local MT5 from the desktop app
- Sign-in providers other than Google; email and password
- Watchlists, alerts, shared layouts, social features
- More than one server instance (no Redis)
- A new visual style for the new screens (direction `hub-native`)

## Constraints

### Technical Constraints
- One router (`hub-core`) shared by every host; one port (8770) for HTTP and `/ws`
- `hub-server` binds `127.0.0.1`; ingress only through the Cloudflare Tunnel
- Migrations stay in the Postgres subset pgsqlite supports; CI runs pgsqlite and Postgres 17
- `MetaTrader5` is Python-only: `bridge/mt5_bridge.py` stays Python
- Google forbids OAuth inside webviews: desktop sign-in goes through the system browser
- New screens use only the hub theme variables from `hub/theme.js` (graphite, black, light presets)
- Repo house rules: no emoji, no em or en dashes, no native `<select>`, no one-sided coloured borders
- AGPL-3.0: the hosted site must offer its exact source

### Business Constraints
- Public launch (Phase 4) is blocked on: data redistribution terms (Q2), the trademark question (Q3),
  privacy text and contact (Q9)
- Hosting region must reach Binance and Bybit public endpoints (Q1)
- Code-signing certificates decide signed vs unsigned desktop builds (Q4)

## Key Decisions

| Decision | Rationale | Date | Status |
|----------|-----------|------|--------|
| Hosted at vtradingview.com for public users | product goal moves from self-hosted tool to public service | 2026-11-23 | Active |
| Axum replaces the Python hub | owner choice; one static binary, one language with the Tauri shell (no bottleneck was measured) | 2026-11-23 | Active |
| Desktop is a thin Tauri shell over the hosted site | owner choice; one router cannot drift between hosts | 2026-11-23 | Active |
| Charts public, saved state behind Google sign-in | lower friction; the database stores only what is worth syncing | 2026-11-23 | Active |
| Server-side sessions, not JWT | revocable; one cookie for HTTP and the websocket upgrade | 2026-11-23 | Active |
| Sign-in merge: server wins, local kept as a 30-day backup | localStorage has no write times; neither side is lost | 2026-11-23 | Active |
| sqlx Postgres driver over pgsqlite by default | one SQL dialect; production switch by `DATABASE_URL` | 2026-11-23 | Active |
| MT5 via per-user HTTPS bridge URLs behind an SSRF guard (option c) | works on web and desktop | 2026-11-23 | Active |
| Logs and restart admin-only (`ADMIN_EMAILS`) | they expose server internals | 2026-11-23 | Active |
| gaffa plus a new Cloudflare Tunnel | proven in pricehub; no inbound ports | 2026-11-23 | Active |
| Public deploy before sync, desktop and MT5 | later phases ship onto a live site | 2026-11-23 | Active |
| New screens in the hub theme, 12 px labels and 14 px text | owner fixed the direction; text floor from the D4 lint | 2026-11-23 | Active |
| Local mode for charts only (D-001) | dev and CI need it anyway; keeps the AGPL self-hosted promise; MT5 returns via Phase 07 bridges | 2026-11-23 | Active |
| Sign-in control sticky-free: scrollable bar content plus an outside account slot | a sticky overlay covered the bar's own buttons at three widths (WCAG 2.4.11) | 2026-11-24 | Active |
| Delete confirmation uses the native `<dialog>` modal | the browser owns the focus trap, Esc and top layer; the modern-web-guidance rule for destructive actions | 2026-11-24 | Active |
| The test-IdP router needs feature, debug profile and `VTV_TEST_IDP=1` | it mints a token for any `sub`; a release build must refuse it, verified 404 | 2026-11-24 | Active |
| Desktop grants the remote page one permission; the deep link arrives by a Rust-side eval (D-029/D-030) | every granted permission is attack surface on an untrusted page; the crate stays its own workspace so the root test job stays tauri-free | 2026-11-25 | Active |

## Success Metrics

| Metric | Target | Current | Status |
|--------|--------|---------|--------|
| p75 time to first live candle | under 2 s | - | Not started |
| Cross-user data exposure | zero (blocks release) | - | Not started |
| Contract suite against Python and Rust | 100% pass | - | Not started |
| Order-flow golden parity | all fixtures within tolerance | - | Not started |
| SSRF suite | every case refused | - | Not started |
| Signed-in users per week | tracked, target deferred | - | Not started |

## Tech Stack / Tools

| Layer | Technology | Notes |
|-------|------------|-------|
| Frontend | existing `hub/` vanilla JS, `vela.bundle.js`, Lightweight Charts, KLineChart | websocket URL, auth UI, sync layer and new screens only |
| Backend | Rust, Axum 0.8, tokio, tokio-tungstenite, reqwest, tracing | crates `hub-types`, `hub-core`, `hub-server` |
| Database | sqlx (postgres driver) over pgsqlite (`~/project/pgsqlite`, baysao fork) or Postgres 17 | `DATABASE_URL` |
| Auth | Google Identity Services (One Tap, redirect, desktop handoff), server-side sessions | |
| Desktop | Tauri 2 with deep-link and opener plugins | `crates/desktop` |
| Process mgmt | gaffa with a `Procfile` (`db`, `hub`, `tunnel`) | pattern from `~/project/pricehub` |
| Edge | Cloudflare Tunnel | new tunnel `vtradingview` |
| MT5 | `bridge/mt5_bridge.py` with `BRIDGE_TOKEN` | per-user, HTTPS |

## PAUL Configuration

Mode `full`: pstack on (stage `plan`: architect, interrogate, swarm while planning), reflect `pstack` at
each phase close then `rsi`, learnings on, Security Audit on (profile `standard`), uxui kit preset on.
See `.paul/config.md` and `.paul/SPECIAL-FLOWS.md`.

---
*Created: 2026-11-23 (imported from .paul/PLANNING.md and .paul/design/specs/vtradingview.md)*
*Last updated: 2026-11-26 after Phase 06*
