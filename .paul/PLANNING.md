---
description: "Hosted, multi-user TradeX at vtradingview.com: Axum backend, Google sign-in, synced state, Tauri desktop shell"
type: Planning
about: "vtradingview"
project_type: "application"
seed_version: "1.0.0"
seed_source: "https://chrisai.cv/skool"
---

# vtradingview

> Building a hosted, Rust-backed TradeX for public traders, so they get free multi-feed charts in a browser or desktop app with their layouts and scripts synced to a Google account: live chart in under 2 s at p75, zero cross-user data exposure.

**Created:** 2026-11-23
**Type:** Application
**Branch:** `arch/axum-tauri`
**Stack:** Rust (Axum 0.8, tokio, sqlx/postgres) + existing vanilla JS frontend (Vela, Lightweight Charts, KLineChart) + Postgres wire protocol via pgsqlite (SQLite file by default, real Postgres in production) + Tauri 2 desktop shell + gaffa process management + Cloudflare Tunnel
**Skill Loadout:** PAUL (required), AEGIS (post-build audit), design-flow / `/seed design` (auth and settings screens), security-review (auth, SSRF), postgres-pro (pgsqlite parity)
**Quality Gates:** contract tests vs Python reference, order-flow golden parity, CI on pgsqlite AND Postgres 17, security review per auth/SSRF phase, repo design gates on new UI, p75 time to first chart

---

## Problem Statement

### Who Has This Problem?
Public traders who want TradingView-style charts on free feeds (Deriv, Bybit, Binance) without installing anything, plus the current TradeX author/users who today must run a local Python hub.

### What Is the Problem?
TradeX only runs as a self-hosted Python process on `127.0.0.1`. To use it, a person needs Python 3, a venv, numpy/pandas/pyarrow, and a terminal. Everything they build (layouts, Pine scripts, favourite tools, indicators, theme) lives in one browser's `localStorage` and is lost on another device or a cleared profile. There is no hosted version and no desktop installer.

### Why Is It Painful?
- **User impact:** setup friction blocks non-technical traders entirely; work is trapped in one browser.
- **Business impact:** no path to a public audience; the Python stack (pandas in the request path, two ports, a `ThreadingHTTPServer`) is not built for many concurrent users.

### Evidence

| Source | Finding |
|--------|---------|
| Repo `README.md`, `AGENTS.md` | Quick start requires Python 3 + venv + `pip install -r requirements.txt`; binds `127.0.0.1` only |
| Frontend source (`hub/*.js`) | Saved state lives only in `localStorage` keys `chartHub`, `chartHubTheme`, `chartHubScripts`, `chartHubIndicators`, `chartHubFavs`, `chartHubDraft` |
| `hub/server.py` | Two ports (HTTP 8770, WS 8765), stdlib HTTP server, numpy/pandas order flow |
| Demand for a hosted version | Assumption: public users want this; not yet validated with users |

### Target Users

| Persona | Context | Primary Goal | Key Pain Point |
|---------|---------|--------------|----------------|
| Public chart user | Retail trader, non-technical, browser or desktop | Open charts on free feeds and keep their setup across devices | Cannot run a Python hub; loses layouts between browsers |
| MT5 power user | Has an MT5 terminal on a VPS | See their own broker's candles in the same UI | Today needs a personal SSH tunnel and local hub |
| Admin (project owner) | Runs the service | See feed health, restart feeds, keep it up | No hosted ops view; logs are local |

---

## Success Metrics

### Primary Metric
**p75 time to first live chart**: from `hub.html` navigation start to the first live candle update rendered on the default chart, measured in the browser.
- **Current:** none (no hosted version)
- **Target:** under 2 s at p75
- **Measured:** a small client beacon (`POST /api/metrics/ttfc`, aggregate only, no user id) reported on the admin panel; checked weekly from Phase 4

### Secondary Metrics
- Signed-in users per week: none, then tracked from Phase 3 (no target set; growth target deliberately deferred)
- Feed uptime per source (Deriv, Bybit, Binance): tracked from Phase 1 on the admin panel

### Guardrail Metrics
- **Zero cross-user data exposure**: no request ever returns another user's `user_state` or `mt5_bridges` row. Enforced by authz tests on every `/api/me/*` route.
- Existing chart behaviour unchanged: the Phase 1 contract suite stays green through every later phase.
- No SSRF: the Phase 7 SSRF suite stays green.

---

## Tech Stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Frontend | Existing `hub/` vanilla JS + `vela.bundle.js` (unchanged except websocket URL, auth UI, sync layer) | Rewriting the frontend (e.g. Yew, as the reference template uses) doubles risk; the contract stays fixed |
| Backend | Rust, Axum 0.8 on tokio; `tokio-tungstenite` (upstream feeds), `reqwest`, `zip`, `tower-http` (static, CSP, compression), `tracing` | One binary, one port, async fan-out to many users; replaces `hub/server.py` and the numpy/pandas order flow |
| Database | sqlx with the **postgres** driver only, pointed by `DATABASE_URL` at pgsqlite (default, SQLite file) or real Postgres | Same SQL everywhere; switching to production Postgres is a config change. pgsqlite from `~/project/pgsqlite` (baysao fork, branch `fix/insert-select-returning-timestamptz`) |
| Cache | In-process (bars LRU, order-flow archive cache on disk) | One server; no Redis until there is more than one instance |
| Auth | Google Identity Services (One Tap + redirect OAuth), server-side sessions | "Auto detect and auto login/create user"; revocable sessions |
| Desktop | Tauri 2, thin shell loading `https://vtradingview.com/hub.html`; deep-link plugin; opener plugin | Same site in a native window; no embedded backend needed |
| Process mgmt | gaffa with a `Procfile` (pattern from `~/project/pricehub/Procfile` + `scripts/dev.sh`) | Already proven alongside pgsqlite in pricehub |
| Edge | Cloudflare Tunnel (`cloudflared`, new tunnel) | TLS at Cloudflare, no inbound ports on the origin, trusted `CF-Connecting-IP` |
| MT5 | `bridge/mt5_bridge.py` stays Python (MetaTrader5 is Python-only), gains token + HTTPS mode | Server calls each user's own bridge (option c) |

### Workspace Layout

```
Cargo.toml                 workspace
crates/hub-types           API + websocket message shapes (the frozen contract), serde
crates/hub-core            axum router, static allow-list, feeds, order flow, log bus,
                           auth, sessions, user state, MT5 bridge client + SSRF guard
crates/hub-server          bin: binds 127.0.0.1:8770, runs migrations, serves everything
crates/desktop             Tauri 2 app (thin shell)
migrations/                sqlx migrations, Postgres subset that pgsqlite supports
hub/                       frontend (unchanged structure)
bridge/                    mt5_bridge.py
Procfile                   gaffa: db, hub, tunnel
scripts/dev.sh             service table (ported from pricehub)
```

### Reference Material
- `~/project/tauri/rust-full-stack-with-auth-template`: take the workspace split, shared types crate, Tauri `custom-protocol` feature, auth handling ideas. Do **not** copy its two drifting backends (`backend` on axum 0.5, `server` on axum 0.7); this project has exactly one router.
- `~/project/pricehub/scripts/dev.sh`: gaffa + pgsqlite service table, port checks, Windows `bash` line quirk (do not pass `gaffa --shell bash` on Windows).

### Research Needed
- pgsqlite coverage for the exact migrations and queries used (UUID, JSON stored as text, `ON CONFLICT ... DO UPDATE ... RETURNING`, `timestamptz`). Answered by Phase 3 CI running both engines.
- Tauri 2 deep-link plugin behaviour on Windows, macOS and Linux for `vtradingview://auth`.
- Which storage keys `vela.bundle.js` writes (layouts, drawings), inventoried in Phase 5.

---

## Data Model

### Entities

| Entity | Key Fields | Relationships |
|--------|-----------|---------------|
| users | `id uuid PK`, `email`, `email_verified bool`, `name`, `avatar_url`, `created_at`, `last_login_at`, `deleted_at` | has many identities, sessions, user_state, mt5_bridges |
| identities | `id`, `user_id FK`, `provider` ('google'), `subject` (Google `sub`), `created_at`; `UNIQUE(provider, subject)` | belongs to user |
| sessions | `id` (random 256-bit, stored as SHA-256 hash), `user_id FK`, `csrf_token_hash`, `created_at`, `expires_at`, `last_seen_at`, `user_agent`, `revoked_at` | belongs to user |
| user_state | `user_id FK`, `key` (`layouts`, `scripts`, `indicators`, `favs`, `theme`, `draft`, plus Vela keys found in Phase 5), `value` (text holding JSON on both engines), `version int`, `updated_at`; `PK(user_id, key)` | belongs to user |
| mt5_bridges | `id`, `user_id FK`, `label`, `url` (https only), `token_ciphertext`, `token_nonce`, `last_ok_at`, `last_error`, `created_at` | belongs to user |
| desktop_auth_codes | `code_hash`, `session_id FK`, `expires_at` (60 s), `used_at` | one-time handoff for Tauri sign-in |

### Notes
- Auto create: sign-in upserts `identities` on `(google, sub)`; a new `sub` creates a new `users` row. Email is stored, never used to join accounts.
- `user_state` is key-value on purpose: it mirrors the existing `localStorage` keys so the frontend sync layer is thin. `version` gives optimistic concurrency (`If-Match`), a stale write returns 409.
- Account deletion is a hard delete of the user and cascade (GDPR); `deleted_at` exists only for the short grace window, if one is added.
- Bridge tokens are encrypted with XChaCha20-Poly1305 under `VTV_SECRET_KEY` from `.env`.
- Migrations stay in the Postgres subset pgsqlite supports; every migration runs in CI on both engines.

---

## API Surface

### Auth Strategy
Server-side sessions in a `__Host-vtv_session` cookie (`HttpOnly`, `Secure`, `SameSite=Lax`, 30-day sliding). Chosen over the template's JWTs because sessions are revocable and the same cookie authenticates the `/ws` upgrade, so the template's first-message websocket handshake is not needed. Writes require an `X-CSRF-Token` header bound to the session and an `Origin` of `https://vtradingview.com`.

Sign-in paths:
- **Web, returning Google user:** Google One Tap prompts automatically; the ID token is POSTed to `/auth/google/onetap`.
- **Web, fallback:** "Sign in with Google" button, redirect flow with `state` + `nonce` + PKCE.
- **Desktop:** Google blocks OAuth inside embedded webviews (`disallowed_useragent`), so the Tauri app opens the system browser at `/auth/google/start?client=desktop`; the callback issues a 60 s one-time code and redirects to `vtradingview://auth?code=...`; the app's window then POSTs it to `/auth/desktop/exchange` and receives the session cookie.

### Route Groups

| Group | Methods | Auth | Purpose |
|-------|---------|------|---------|
| `/hub.html`, static allow-list | GET | public | Frontend; everything not on the list is 404 (`.env`, `.paul/`, sources) |
| `/api/status`, `/api/health`, `/api/symbols`, `/api/instruments`, `/api/bars` | GET | public, IP rate-limited | Existing contract, unchanged shapes |
| `/api/orderflow/{delta,footprint,profile}` | GET | public, stricter rate limit | Existing contract |
| `/ws` | GET (upgrade) | public; session attached if cookie present | Live data, replaces port 8765; 30 s ping for Cloudflare idle timeout; subscription cap per connection |
| `/auth/google/onetap` | POST | public | Verify ID token (JWKS signature, `aud`, `iss`, `exp`), upsert user, set cookie |
| `/auth/google/start`, `/auth/google/callback` | GET | public | Redirect flow (web fallback, desktop) |
| `/auth/desktop/exchange` | POST | one-time code | Desktop session handoff |
| `/auth/me`, `/auth/logout` | GET, POST | session | Current user, sign out |
| `/api/me/state/:key` | GET, PUT | session + CSRF | Synced state, `If-Match: <version>` |
| `/api/me` | DELETE | session + CSRF | Account deletion |
| `/api/me/mt5` , `/api/me/mt5/:id`, `/api/me/mt5/:id/test` | GET, POST, DELETE, POST | session + CSRF | Manage per-user MT5 bridges |
| `/api/bars?source=mt5&bridge=:id` | GET | session (owner only) | MT5 candles through the SSRF guard |
| `/api/metrics/ttfc` | POST | public, rate-limited | Aggregate time-to-first-chart beacon |
| `/admin/logs`, `/admin/restart`, `/admin/metrics` | GET, POST | admin session (`ADMIN_EMAILS` + verified Google email) | Log panel, feed restart, metrics; non-admins get 404 |

### Internal vs External
- **Public endpoints:** static frontend, market-data `/api/*`, `/ws`, `/auth/*`, metrics beacon.
- **Internal/admin endpoints:** `/admin/*` (replaces `/api/logs*` and `/api/restart`, which are removed from the public surface).
- **MCP integration points:** none in the hosted service. The MT5 bridge talks to the MT5 terminal's MCP endpoint on the user's own machine, unchanged.

---

## Deployment Strategy

### Local Development
gaffa runs a `Procfile` whose lines all go through `scripts/dev.sh` (ported from pricehub). Default dev database is a pgsqlite SQLite file.

| Service | Image/Runtime | Port | Purpose |
|---------|--------------|------|---------|
| db | pgsqlite (built from `~/project/pgsqlite`, `cargo build --release`) `--database data/vtv.db --port 55432` | 55432 | Postgres wire protocol over SQLite |
| hub | `cargo run -p hub-server` (dev) / release binary (prod) | 8770 (127.0.0.1) | Axum: frontend, API, websocket, auth |
| tunnel | `cloudflared tunnel run vtradingview` | none inbound | Production only; public HTTPS for vtradingview.com |
| bridge (optional, dev) | `python bridge/mt5_bridge.py` with `BRIDGE_TOKEN` | 8797 | Local MT5 bridge for testing Phase 7 |

`.env` (never committed): `DATABASE_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `VTV_SECRET_KEY`, `ADMIN_EMAILS`, `PUBLIC_ORIGIN=https://vtradingview.com`, `TUNNEL_TOKEN`.

### Staging / Production
- One Linux host in a region where Binance and Bybit public endpoints are not blocked (not US). gaffa supervises `db` (or nothing, once `DATABASE_URL` points at managed Postgres), `hub`, `tunnel`.
- A new Cloudflare Tunnel routes `vtradingview.com` to `http://127.0.0.1:8770`. The host opens no inbound ports.
- CI (GitHub Actions): `cargo test` (unit + contract suite), migrations and auth tests against **pgsqlite and Postgres 17**, `cargo clippy -D warnings`, `cargo audit`, Tauri builds for Windows, macOS, Linux on tags.
- Backups: nightly copy of the SQLite file (with pgsqlite paused or via SQLite backup API) or `pg_dump` once on Postgres; restore drill once before launch.
- The Python server stays runnable on `main` for self-hosters until Phase 2 passes parity, then is retired on this branch.

---

## Security Considerations

- **Auth/Authz model:** Google-only sign-in; identity is Google `sub`. Sessions are random, stored hashed, revocable; every `/api/me/*` query is scoped by `session.user_id` in SQL, never by a client-supplied id. Admin is `ADMIN_EMAILS` and `email_verified`; admin routes return 404 to everyone else.
- **Google token checks:** signature against Google JWKS (cached, refreshed on key-id miss), `aud` equals our client id, `iss` is Google, `exp` valid, `nonce` matches on the redirect flow, `state` + PKCE on the redirect flow.
- **CSRF:** `SameSite=Lax` cookie plus per-session `X-CSRF-Token` on every non-GET plus `Origin` check.
- **Input validation:** typed extractors for every query (`source`, `symbol`, `tf`, `limit` bounded); `user_state` value size cap (e.g. 1 MB per key) and JSON validity; MT5 `url` must be `https://`.
- **OWASP concerns specific to this app:**
  - **SSRF (Phase 7):** the server calls user-supplied bridge URLs. Resolve the host, reject loopback, private (RFC 1918, ULA), link-local (incl. `169.254.169.254`), CGNAT and multicast ranges, then connect to the pinned resolved IP (defeats DNS rebinding), no redirects, 10 s timeout, 5 MB response cap.
  - **Broken access control:** cross-user tests on every `/api/me/*` route (guardrail metric).
  - **XSS:** strict CSP (self + Google Identity Services origins), no inline script additions; Pine scripts are user content and stay sandboxed to the owner's own session.
  - **Static file exposure:** allow-list only; `.env`, `.paul/`, `crates/`, `bridge/` always 404.
- **Desktop (Tauri):** the window loads a remote origin, so it gets **no** Tauri IPC capabilities except opening the system browser for sign-in and the deep-link listener. CSP and `dangerousRemoteDomainIpcAccess`-style grants stay off.
- **Secrets management:** `.env` on the host only (already gitignored); bridge tokens encrypted at rest with `VTV_SECRET_KEY`; nothing secret in logs (the existing redaction in `logbus` is ported).
- **Rate limiting:** per IP via `CF-Connecting-IP` (trusted because only the tunnel reaches the origin), stricter per user on order flow and MT5, websocket subscription cap per connection, upstream feeds shared (one subscription per symbol) so user count does not multiply upstream load.
- **Privacy:** only email, name, avatar stored; account deletion endpoint; privacy page before public launch (public EU users, GDPR).

---

## UI/UX Needs

### Design System
The existing hub UI and themes stay. New screens follow the repo's design rules (one token source, all eight interaction states, keyboard parity, no native `<select>`, no emoji, WCAG AA in light and dark). They go through `/seed design` before they are built.

### Key Views / Pages

| View | Purpose | Complexity |
|------|---------|------------|
| Hub header: sign-in | One Tap prompt + "Sign in with Google" button; after sign-in, avatar menu (sync status, MT5 bridges, delete account, sign out) | medium |
| Sync status / conflict notice | "Saved to your account", and a clear message with a reload action when another device saved first (409) | low |
| Settings: MT5 bridges | Add URL + token, Test with a real result, per-bridge status and last error, delete | medium |
| Delete account dialog | Destructive confirm (destructive button styling in trigger and confirm) | low |
| Admin log panel | Existing log panel, now admin-only, plus feed health and TTFC metric | medium (real-time) |
| Desktop sign-in waiting state | "Finish signing in in your browser" with retry and cancel | low |
| Privacy page | Static page | low |

### Real-Time Requirements
Existing websocket streams (candles, order flow) on `/ws`; admin log panel streams over the same socket on an admin-only channel.

### Responsive Needs
Desktop-first charting app (unchanged). New dialogs and menus are mobile-first and must pass the repo responsive gate (280 to 951 px).

---

## Integration Points

| Integration | Type | Purpose | Auth |
|------------|------|---------|------|
| Deriv | websocket in | Forex, gold, indices, crypto candles | none; reconnect with backoff, shown as stale on failure |
| Bybit | websocket + REST in | About 3,300 crypto and metal markets | none; same failure handling |
| Binance | websocket + REST + trade archives in | Crypto candles, order-flow trades | none; order flow falls back to the labelled candle estimate |
| Google Identity Services | OAuth / OIDC both ways | Sign-in, auto create user | client id + secret; site works signed-out if Google is down, saving disabled |
| Users' MT5 bridges | HTTPS out | Per-user broker candles | per-bridge bearer token; failure affects only that user |
| Cloudflare Tunnel | ingress | Public HTTPS | tunnel token; gaffa restarts `cloudflared` |
| pgsqlite / Postgres | SQL both ways | Users, sessions, state, bridges | `DATABASE_URL`; `/api/health` reports not-ready, signed-in features fail cleanly |

---

## Phase Breakdown

### Phase 1: Rust workspace and hub-server parity
- **Build:** Cargo workspace (`hub-types`, `hub-core`, `hub-server`), static allow-list, every existing `/api/*` route except order flow, `/ws` merged onto port 8770, Deriv/Bybit/Binance feeds with one shared upstream per symbol, log bus with redaction, gaffa `Procfile` + `scripts/dev.sh` (`hub` line). Frontend change: websocket URL derived from `location`.
- **Testable:** a contract suite (the `AGENTS.md` checks plus response shapes for every route and websocket message) runnable against any base URL.
- **Outcome:** the existing UI runs on the Rust server with free feeds.
- **Accept:** the contract suite passes against **both** `python hub/server.py` and `hub-server`; `/.env` returns 404; one port only.

### Phase 2: Order flow in Rust
- **Build:** delta, footprint, session volume profile and TPO computations, Binance trade archive download and cache (`zip` + `reqwest`), candle-based estimate fallback.
- **Testable:** golden fixtures: Python outputs captured on fixed inputs, compared with Rust outputs.
- **Outcome:** order-flow chart types work on `hub-server`; Python, numpy, pandas and pyarrow are no longer needed for the hub.
- **Accept:** every golden fixture matches within a stated numeric tolerance (e.g. 1e-9 relative); `requirements.txt` keeps only the bridge's needs.

### Phase 3: Database and auth
- **Build:** sqlx migrations (users, identities, sessions, desktop_auth_codes), `db` gaffa line running pgsqlite, Google One Tap + redirect flow, sessions, CSRF, `/auth/*`, admin gate (`/admin/*` replaces `/api/logs*` and `/api/restart`), `DELETE /api/me`.
- **Testable:** migration and auth integration tests on pgsqlite and Postgres 17 in CI; Google token verification unit tests with forged tokens (bad `aud`, expired, wrong signature).
- **Outcome:** a visitor can sign in with Google and is auto-created; the admin sees the log panel, nobody else does.
- **Accept:** first Google sign-in creates exactly one user, second sign-in reuses it; CI green on both database engines; non-admin gets 404 on `/admin/*`.

### Phase 4: Public deploy
- **Build:** production host, new Cloudflare Tunnel (`tunnel` gaffa line), rate limits, CSP and security headers, backups and restore drill, licence attribution notices for bundled chart libraries on the site, privacy page, TTFC beacon and admin metric.
- **Testable:** external smoke test against `https://vtradingview.com`; port scan of the host shows no inbound ports.
- **Outcome:** `https://vtradingview.com/hub.html` is live with charts and Google sign-in.
- **Accept:** live charts load through the tunnel; host has no open inbound ports; backup restore drill succeeded; TTFC visible on the admin panel.

### Phase 5: Synced state
- **Build:** `user_state` migration, `/api/me/state/:key` with versioning, sign-in merge (server wins, local value kept as a 30-day backup), a frontend sync layer over the existing `localStorage` keys plus the Vela keys found by inventory, conflict notice UI, sign-in UI in the hub header.
- **Testable:** cross-user authz tests; conflict test (two writers, second gets 409); sign-in merge and backup restore test.
- **Outcome:** a user's layouts, scripts, indicators, favourites and theme follow them across browsers.
- **Accept:** a layout drawn signed-out appears after sign-in in a second browser; cross-user tests all pass.

### Phase 6: Tauri desktop
- **Build:** `crates/desktop` thin shell loading the hosted `hub.html`, deep-link plugin (`vtradingview://auth`), opener plugin, desktop sign-in waiting state, tray, locked-down capabilities, installers for Windows, macOS, Linux in CI.
- **Testable:** capability audit (remote origin has no IPC beyond opener/deep-link); sign-in round trip on each OS.
- **Outcome:** an installable desktop app showing the same account and layouts.
- **Accept:** the installed app signs in via the system browser and shows the same synced layout as the web.

### Phase 7: Per-user MT5 bridges
- **Build:** `BRIDGE_TOKEN` + HTTPS-ready mode in `bridge/mt5_bridge.py` (with a documented Cloudflare Tunnel or Caddy setup for users), `mt5_bridges` migration with encrypted tokens, SSRF guard, `/api/me/mt5*`, `source=mt5&bridge=:id`, settings screen.
- **Testable:** SSRF suite (loopback, `169.254.169.254`, RFC 1918, IPv6 ULA, DNS rebinding name, redirect to private IP) all refused; owner-only access tests.
- **Outcome:** an MT5 user sees their own broker's candles on vtradingview.com.
- **Accept:** a real bridge streams candles for its owner; every SSRF case is refused; another user cannot use or see the bridge.

---

## Out of Scope

| Excluded | Why Not Now | Revisit When |
|----------|-------------|--------------|
| Order placement / trading | Project rule: read-only market data | Only on an explicit request |
| Frontend rewrite (Yew, React, Vite) | Doubles risk; contract is frozen | A phase is blocked by the vanilla JS frontend |
| Other sign-in providers | Google covers the brief | User demand; `identities` already supports it |
| Desktop offline mode / embedded local backend / local MT5 | Desktop is a thin shell over the hosted site | Bridge setup proves too hard for users |
| Watchlists, alerts, shared layouts | Not in the brief | After Phase 5 ships and users ask |
| Multi-instance scaling (Redis, sticky websockets) | One host is enough at launch | Sustained CPU or connection pressure on one host |
| Growth target for signed-in users | Deferred by decision | After 4 weeks of live data |

---

## Dependencies & Risks

### Dependencies

| Dependency | Type | Owner | Needed By |
|-----------|------|-------|-----------|
| Google OAuth client (web) with `https://vtradingview.com` origin and callback | external | project owner | Phase 3 |
| Cloudflare account + new tunnel for `vtradingview.com` | external | project owner | Phase 4 |
| Production Linux host (non-US region) | external | project owner | Phase 4 |
| pgsqlite release build from `~/project/pgsqlite` (`fix/insert-select-returning-timestamptz`) | technical | project owner | Phase 3 |
| Apple Developer ID / Windows code-signing certificate | external | project owner | Phase 6 (unsigned builds work for testing) |
| gaffa installed on dev and prod hosts | technical | project owner | Phase 1 |

### Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| pgsqlite (experimental) mishandles a query or migration | med | high | CI runs every migration and query on pgsqlite and Postgres 17; production switches `DATABASE_URL` to Postgres the day it misbehaves |
| Upstream feeds throttle or ban the single server IP | med | high | One shared upstream per symbol, bars cache, backoff, per-user limits, admin feed-health view |
| Binance or Bybit blocks the hosting region | med | high | Choose a non-US region; verify endpoints from the host before Phase 4 sign-off |
| Data redistribution terms of Deriv, Bybit, Binance forbid public serving | med | high | Read each provider's API terms before Phase 4 launch; drop or gate a feed that forbids it |
| "vtradingview" name conflicts with the TradingView trademark | med | high | Legal review before Phase 4 public launch; keep the non-affiliation notice; have an alternate name ready |
| Google blocks sign-in inside the Tauri webview | high (by policy) | high | System-browser + deep-link flow designed in from the start (Phase 6) |
| Order-flow numbers drift from the Python version | med | med | Golden fixtures with an explicit tolerance gate in Phase 2 |
| SSRF through user bridge URLs | med | high | Resolve-and-pin guard, deny private ranges, no redirects, size and time caps, dedicated test suite |
| Remote page in Tauri gains IPC access | low | high | Capabilities grant nothing to the remote origin beyond opener and deep link; audited in Phase 6 |
| Licence attribution missing on the hosted site | med | med | Attribution notices shipped as a Phase 4 deliverable |

---

## Skill Loadout & Quality Gates

### Skills Used During Build

| Skill | When It Fires | Purpose |
|-------|--------------|---------|
| PAUL (`/paul:init`, `/paul:plan`, `/paul:apply`, `/paul:verify`) | All phases | Managed milestones and phases |
| `/seed design` (design-flow) | Before Phases 5, 6, 7 UI work | Sign-in menu, sync notice, MT5 settings, desktop waiting state |
| security-review / AEGIS (`/aegis-audit`) | End of Phases 3, 4, 7 | Auth, deployment, SSRF review |
| postgres-pro | Phase 3 | Migrations that work on pgsqlite and Postgres |
| gaffa | Phases 1, 3, 4 | Procfile and service table |
| tauri-* skills (deep link, capabilities, code signing) | Phase 6 | Desktop shell |

### Quality Gates

| Gate | Threshold | When |
|------|-----------|------|
| Contract suite vs Python reference | 100% pass on both servers | Phase 1, then every phase |
| Order-flow golden parity | all fixtures within tolerance | Phase 2, then every phase |
| Dual-engine DB tests | green on pgsqlite and Postgres 17 | Phase 3 onward |
| Cross-user authz tests | zero leaks | Phase 3 onward |
| SSRF suite | every case refused | Phase 7 onward |
| `cargo clippy -D warnings` + `cargo audit` | clean | every phase |
| Repo design gates on new UI (`node scripts/accuracy_report.mjs` where applicable) | all pass, light and dark | UI phases |
| Performance | p75 time to first live chart under 2 s | Phase 4 onward |

---

## Design Decisions

1. **One router, two hosts**: a single `hub-core` router serves web; the desktop app is a thin shell over the hosted site. Rationale: the reference template's two backends drifted (axum 0.5 vs 0.7); one router cannot drift.
2. **Hosted at vtradingview.com for public users**: the product goal moves from self-hosted tool to public service.
3. **Frontend kept, contract frozen**: existing `/api/*` and websocket message shapes stay; only the websocket URL, auth UI and sync layer change.
4. **One port**: HTTP and websocket merge onto 8770 (`/ws`), so pages work same-origin in browser and desktop.
5. **Google-only sign-in with auto create**: One Tap on web, redirect fallback, system browser + deep link on desktop (Google forbids OAuth in webviews). Identity keyed on Google `sub`.
6. **Server-side sessions, not JWT**: revocable, one cookie for HTTP and websocket.
7. **Charts public, saved state behind login**: lower friction; the database stores only what is worth syncing. Anonymous state uploads automatically on first sign-in.
8. **sqlx postgres driver over pgsqlite by default**: SQLite file in dev and early prod, real Postgres by `DATABASE_URL`; both tested in CI.
9. **Key-value `user_state` with versions**: mirrors existing `localStorage` keys; 409 on conflicting writes.
10. **MT5 option (c), per-user bridge URL**: the server calls each user's own HTTPS bridge with a bearer token, behind an SSRF guard. The bridge stays Python.
11. **Logs and restart are admin-only**: they expose server internals; non-admins get 404.
12. **gaffa + new Cloudflare Tunnel**: process management as in pricehub; no inbound ports; TLS at Cloudflare.
13. **Deploy early (Phase 4) before sync, desktop and MT5**: later phases ship onto a live site.
14. **Primary metric p75 time to first live chart under 2 s; guardrail zero cross-user exposure**: measurable from day one; growth target deferred.

---

## Open Questions

1. Which hosting provider and region for the production host (must reach Binance and Bybit public endpoints)?
2. Do the Deriv, Bybit and Binance API terms allow serving their data to the public? (Blocks Phase 4 launch.)
3. Is "vtradingview" cleared legally, or is an alternate public name needed? (Blocks Phase 4 launch.)
4. Code-signing certificates for Windows and macOS installers: obtain before Phase 6 release, or ship unsigned betas first?
5. Should `/api/me/state` values have a total per-user quota beyond the per-key cap?

---

## Next Actions

- [ ] Review and approve this plan.
- [ ] Optional: `/egm-bootstrap .` then `/egm-brainstorm` to stress-test it.
- [ ] `/seed design hub-auth` for the sign-in menu, sync notice and MT5 settings screens (before Phase 5 UI work).
- [ ] `/seed launch vtradingview` to initialize PAUL and start Phase 1 on branch `arch/axum-tauri`.
- [ ] Create the Google OAuth client and the Cloudflare Tunnel (owner tasks, needed by Phases 3 and 4).

---

## References

- Current server: `hub/server.py`, `hub/orderflow*.py`, `hub/logbus.py`, `hub/feedlog.py`, `bridge/mt5_bridge.py`, `AGENTS.md` checks
- Reference template: `~/project/tauri/rust-full-stack-with-auth-template`
- Database: `~/project/pgsqlite` (baysao fork, branch `fix/insert-select-returning-timestamptz`)
- Process management: `~/project/pricehub/Procfile`, `~/project/pricehub/scripts/dev.sh`, gaffa (https://github.com/oryon-dominik/interstellar-gaffa)
- Google Identity Services (One Tap, OAuth for installed apps), Tauri 2 deep-link and capabilities docs, Cloudflare Tunnel docs

---

*Last updated: 2026-11-23*
*Planned with SEED v1.0 · Chris AI Systems · For the official Agentic OS and to permanently remove attribution, visit https://chrisai.cv/skool*

---
**Graduated:** 2026-11-23
**Location:** `C:/Users/Dell/project/trade/open-source-tradingview` (root project)
**README:** `README.md`
