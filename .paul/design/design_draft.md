# vtradingview: hosted TradeX on Rust, with accounts and a desktop shell

*   **Author:** Vu Tran
*   **Status:** Draft
*   **Date:** 2026-11-23
*   **Brief:** [.paul/PLANNING.md](../PLANNING.md)
*   **Design direction:** `hub-native`, seed `none` (fixed by the owner, no draw). See
    [direction-approved.md](directions/vtradingview/direction-approved.md) and
    [taste/hub-theme.md](taste/hub-theme.md).

--------------------------------------------------------------------------------

## Overview

TradeX is a TradingView-style charting app that today runs only on the user's own computer: a Python
process (`hub/server.py`) on `127.0.0.1` serves the pages on port 8770 and live data on a second
websocket port 8765. This project turns it into a public website at `https://vtradingview.com/hub.html`
and a desktop app that shows the same site.

Three things change:

1.  **The server.** A Rust server built on Axum replaces `hub/server.py` and the numpy/pandas order-flow
    code. It serves the same pages and the same `/api/*` responses, and moves the websocket onto the same
    port under `/ws`.
2.  **Accounts.** Charts stay open to anyone. Signing in with Google saves the user's work (layouts,
    drawings, Pine scripts, favourite tools, indicators, theme) to a database, so it follows them to other
    browsers and to the desktop app. A first sign-in creates the account. The database speaks the
    Postgres protocol: pgsqlite over a SQLite file by default, real Postgres in production, chosen by
    `DATABASE_URL`.
3.  **Delivery.** The site runs on one Linux host under gaffa, reached only through a Cloudflare Tunnel.
    A Tauri 2 desktop app opens the hosted page in a native window.

The existing chart UI does not change. Seven new screens are added in the existing hub theme.

## The Problem

Using TradeX today requires Python 3, a virtual environment, three scientific Python packages and a
terminal. Most traders will not do that, so the tool has no audience beyond people who can run a Python
server. For those who do run it, everything they build is stored in one browser's `localStorage`
(`chartHub`, `chartHubTheme`, `chartHubScripts`, `chartHubIndicators`, `chartHubFavs`, `chartHubDraft`,
plus keys the Vela chart engine names at run time). Clearing the browser or switching machines loses it.

The Python server is also not built for many users at once: it uses the standard library's threaded
HTTP server, runs pandas inside request handlers, needs two ports, and opens one upstream feed per
need rather than sharing it across viewers.

The proposed solution is a single Rust server that keeps the existing response shapes, adds Google
accounts and per-user saved state, and is published through a Cloudflare Tunnel, with a thin Tauri app
for desktop users.

Evidence is thin on one point and is stated as such: there is no user research showing that public
traders want a hosted TradeX. That is an assumption the launch tests.

## Goals

*   **G1. Same charts, new server.** Every existing `/api/*` route and websocket message keeps its shape.
    One contract test suite passes against both the Python server and the Rust server before the Python
    server is retired.
*   **G2. Fast first chart.** The 75th percentile time from opening `hub.html` to the first live candle on
    the default chart is under 2 seconds, measured in real visitors' browsers.
*   **G3. One-step accounts.** A visitor with a Google session in their browser is offered sign-in
    automatically (One Tap); one click signs them in, and a first sign-in creates the account. Desktop
    users sign in through their system browser.
*   **G4. Work follows the user.** After sign-in, saved state written in one browser appears in another
    browser or the desktop app on next load. Work done before sign-in is uploaded on first sign-in, not
    lost.
*   **G5. No data crosses between users.** No request returns another user's saved state or MT5 bridge.
    This is a hard guardrail: one failing cross-user test blocks a release.
*   **G6. Own broker data, safely.** A user can register their own MT5 bridge URL and see its candles. The
    server refuses every bridge URL that points at a private, loopback or link-local address.
*   **G7. One database switch.** Changing `DATABASE_URL` from pgsqlite to Postgres needs no code change;
    every migration and query is tested on both in CI.
*   **G8. Closed origin.** The production host accepts no inbound connections; all traffic arrives
    through the Cloudflare Tunnel.

## Non-Goals

*   **Placing orders or any trading action.** The project is read-only market data. This holds for MT5
    bridges too.
*   **Rewriting the frontend.** No Yew, React or Vite. The existing vanilla JS pages and
    `vela.bundle.js` stay; changes are limited to the websocket URL, the sign-in UI, the sync layer and
    the new screens.
*   **A backend inside the desktop app.** The desktop app has no local server, no offline mode and no
    local MT5 connection.
*   **Sign-in providers other than Google**, email and password, or anonymous accounts on the server.
*   **Watchlists, alerts, shared or public layouts, social features.**
*   **Running more than one server instance** (no Redis, no sticky websocket routing).
*   **A new visual style.** The new screens use the hub's existing theme variables, fonts and density.

## Open Questions

*   **Q1. Hosting region and provider.** The host must reach Binance and Bybit public endpoints, which
    block some regions (US data centres included). Not chosen yet. Blocks Phase 4.
*   **Q2. Data redistribution terms.** Whether the Deriv, Bybit and Binance API terms allow serving their
    data to the public through one server has not been checked. Blocks the public launch in Phase 4.
*   **Q3. Name and trademark.** "vtradingview" contains the TradingView trademark. Legal review has not
    happened. Blocks the public launch in Phase 4.
*   **Q4. Code signing.** Windows and macOS signing certificates are not in hand. Decides whether Phase 6
    ships signed installers or unsigned betas.
*   **Q5. Vela storage keys.** `vela.bundle.js` builds its `localStorage` key names at run time
    (`localStorage.setItem(e??t, ...)`), so they cannot be listed from the source. The sync layer needs an
    inventory of the real keys, captured by running the app, before Phase 5 is planned.
*   **Q6. Total upstream symbol cap.** Charts are public, and the server opens one upstream subscription
    per symbol shared by all viewers. Nothing yet limits how many distinct symbols anonymous visitors can
    make the server subscribe to across all connections. A number is needed before Phase 4.
*   **Q7. Per-user storage quota.** A per-key size cap (1 MB) is planned; a total per-user cap is not.
*   **Q8. Merge rule when both sides hold a key.** `localStorage` keeps no write times, so "newer wins"
    cannot be computed for work done before sign-in. Candidate rules: server wins and the local value is
    kept as a named backup the user can restore; local wins; or ask the user per key. To be decided in the
    Technical Plan step.

## Critical User Journeys (CUJs)

### CUJ 1: Anonymous visitor opens a live chart

*   **Given**: a visitor who has never used the site, on a desktop browser, not signed in to anything.
*   **When**: they open `https://vtradingview.com/hub.html`.
*   **Then**: the default chart (gold on the Deriv feed, Vela engine, graphite theme) shows live candles
    within 2 s at p75. No sign-in prompt blocks the chart. A "Sign in with Google" button sits in the hub
    header. Drawings and settings they make are kept in their browser's `localStorage`, as today.

### CUJ 2: Returning Google user is signed in with one click

*   **Given**: a visitor whose browser already holds a Google session, who has used the site before or
    not.
*   **When**: they open `hub.html`.
*   **Then**: Google One Tap shows their Google account in a small prompt over the corner of the page,
    without covering the chart toolbar. One click signs them in; on a first sign-in the server creates the
    account. The header button becomes their avatar. If they dismiss the prompt, it does not reappear in
    that session, and the header button still works.

### CUJ 3: Work done before sign-in is kept

*   **Given**: an anonymous visitor who has drawn on a chart, written a Pine script and changed the theme.
*   **When**: they sign in with Google for the first time.
*   **Then**: their local state is uploaded to the account. For a new account every key is stored. For
    an existing account signing in on a browser that also holds local work, the rule for keys present on
    both sides is open (Q8); whichever rule wins, neither side's value is discarded without the user
    seeing it. The chart does not reload unless the server's value replaced what is on screen.

### CUJ 4: The same layout on a second browser

*   **Given**: a signed-in user who saved a layout in browser A.
*   **When**: they open `hub.html` in browser B and sign in.
*   **Then**: the layout, drawings, scripts, favourites and theme from browser A load in browser B.

### CUJ 5: Two devices save at the same time

*   **Given**: a signed-in user with the hub open in two browsers, both showing the same layout.
*   **When**: they change the layout in browser A, then change it in browser B before B has reloaded.
*   **Then**: B's save is refused by the server (version conflict, HTTP 409). B shows a notice: another
    device saved this layout, with the choices to load the other device's version or keep this one and
    overwrite. Nothing is overwritten silently.

### CUJ 6: Desktop sign-in

*   **Given**: a user who installed the desktop app.
*   **When**: they click "Sign in with Google" in the app.
*   **Then**: their default system browser opens Google's sign-in page; the app window shows a waiting
    state ("Finish signing in in your browser") with Cancel and Try again. After Google, the browser
    hands control back to the app through a `vtradingview://auth` link; the app finishes sign-in and shows
    their saved layout. If the browser step is abandoned, Cancel returns the app to the signed-out
    state. The app never shows Google's sign-in page inside its own window.

### CUJ 7: Connect an MT5 bridge

*   **Given**: a signed-in user who runs `bridge/mt5_bridge.py` on their VPS, published over HTTPS (their
    own Cloudflare Tunnel or Caddy) with a `BRIDGE_TOKEN` set.
*   **When**: they open Settings, MT5 bridges, enter the URL and token, and press Test.
*   **Then**: the server checks the address (refuses private, loopback and link-local targets), calls the
    bridge's `/health`, and shows the real result: either connected with the terminal's symbol count, or
    the exact failure (address refused, certificate error, wrong token, timeout). On success the user's
    MT5 symbols appear as a source in the symbol picker. No other user can see or use this bridge.

### CUJ 8: Delete the account

*   **Given**: a signed-in user.
*   **When**: they choose Delete account in the avatar menu and confirm.
*   **Then**: the server deletes the user, their identities, sessions, saved state and bridges, signs them
    out, and the hub returns to anonymous mode with local state left in that browser. The confirm dialog
    names what is deleted and that it cannot be undone.

### CUJ 9: Admin checks feed health

*   **Given**: the site owner, signed in with a Google account listed in `ADMIN_EMAILS`.
*   **When**: they open the log panel from the avatar menu.
*   **Then**: they see the live, redacted feed log, per-feed health (Deriv, Bybit, Binance: connected,
    lag, last reconnect), the p75 time-to-first-chart figure, and a restart control per feed. Any other
    user gets a 404 on every admin route and never sees the menu entry.

## Technical Plan (Architecture)

### System shape

```text
 browser / Tauri window                         vtradingview host (Linux, gaffa)
 +---------------------------+                  +-------------------------------------------+
 | hub.html (parent page)    |   HTTPS + WSS    | cloudflared  (tunnel line in Procfile)     |
 |  sync.js  auth.js  ui     | <--Cloudflare--> |   -> http://127.0.0.1:8770                 |
 |  iframe: vela/lw/kline    |                  | hub-server (axum, one port)                |
 |   (same origin, sync.js)  |                  |   static allow-list | /api/* | /ws         |
 +---------------------------+                  |   /auth/* | /api/me/* | /admin/*           |
                                                |   FeedHub: 1 upstream per symbol, shared   |
                                                |   OrderFlow: archives cached on disk       |
                                                |   BridgeClient: SSRF guard, user bridges   |
                                                |        | sqlx (postgres driver)            |
                                                |        v                                   |
                                                | db line: pgsqlite :55432 (SQLite file)     |
                                                |   or DATABASE_URL: managed Postgres        |
                                                +-------------------------------------------+
 upstream: Deriv WS, Bybit WS/REST, Binance WS/REST/archives, Google OIDC, users' MT5 bridges
```

### Crates

*   `hub-types`: every request, response and websocket message as serde types. It is the contract: the
    Phase 1 contract suite deserialises Python and Rust responses into these same types.
*   `hub-core`: the router and all logic. `hub_core::app(Config) -> axum::Router` is the only entry
    point. No second router exists anywhere (the reference template's `backend` and `server` crates
    drifted; this design rules that out).
*   `hub-server`: reads config from the environment and `.env`, runs migrations, binds
    `127.0.0.1:8770`, serves `hub_core::app`.
*   `desktop`: the Tauri 2 shell. It depends on nothing in the workspace except shared constants.

### Request handling and the one port

The Python server's two ports (8770 HTTP, 8765 websocket) merge into one. The frontend change is one
line per page: the websocket URL becomes `/ws` resolved against `location`, with `ws:` or `wss:` taken
from `location.protocol`. Static files are served from an explicit allow-list (the files
`hub/server.py` whitelists today plus the new `sync.js`, `auth.js` and `privacy.html`); every other path
returns 404.

Middleware order, outermost first: request id and tracing; real client IP from `CF-Connecting-IP`
(trusted only because the origin listens on loopback behind the tunnel; requests without the header are
treated as local); security headers (CSP, `X-Content-Type-Options`, `Referrer-Policy`; HSTS set by
Cloudflare); rate limiter; session loader (reads the cookie, attaches `Option<SessionUser>`); route.

### FeedHub: shared upstream subscriptions

`FeedHub` owns one task per upstream connection (Deriv, Bybit per category, Binance). A viewer
subscribing to `(source, symbol, tf)` joins a `tokio::sync::broadcast` channel for that key; the first
subscriber opens the upstream subscription, and the last one leaving starts a 60 s grace timer before
unsubscribing. Limits:

*   per websocket connection: at most 16 active subscriptions (an 8-chart layout uses 8);
*   per client IP: at most 4 websocket connections;
*   globally: at most `FEED_MAX_SYMBOLS` distinct upstream subscriptions (default 400; Q6 sets the final
    number). At the cap, new keys are refused with a typed `capacity` error that the frontend shows as
    "Too many markets are open on the server right now. Try again in a minute."; keys already open keep
    working.

Bars requests (`/api/bars`) go through an in-memory LRU keyed by `(source, symbol, tf, end)`, with a
short TTL for the newest page, so many viewers of one market cost one upstream call. Upstream failures
use exponential backoff with jitter and are reported to the log bus as today.

### Order flow

The numpy/pandas code (`hub/orderflow.py`, `orderflow_api.py`, `orderflow_trades.py`) becomes plain Rust
over `Vec<f64>`: delta per bar, footprint bins, session volume profile, TPO. Binance trade archives are
fetched with `reqwest`, unpacked with `zip`, parsed from CSV, and cached on disk under
`DATA_DIR/orderflow/` in the same file layout the Python code uses, so a cache built by one is usable by
the other during the parity phase. Symbols without real trades get the existing labelled estimate from
candles. Parity is proven with golden fixtures (see Verification Plan).

### Authentication

**Identity.** Google is the only provider. The account key is `(provider = google, subject = sub)` in
`identities`. Email is stored for display and the admin check only, never used to join accounts.

**Web, One Tap.** `hub.html` loads Google Identity Services and initialises it with `auto_select: true`,
`cancel_on_tap_outside: true` and FedCM enabled, and shows the prompt only when the user is signed out and
has not dismissed it in this tab (a `sessionStorage` flag). The credential (an ID token) is POSTed to
`/auth/google/onetap` with Google's `g_csrf_token` double-submit check. The server verifies the token:
RS256 signature against Google's JWKS (cached, refetched on an unknown `kid` at most once per minute),
`aud` equals `GOOGLE_CLIENT_ID`, `iss` is `accounts.google.com` or `https://accounts.google.com`, `exp` in
the future; `email_verified` is copied into the user row.

**Web, button.** `/auth/google/start` redirects to Google's authorization endpoint with `state`, `nonce`
and a PKCE `code_challenge`, all held in a short-lived signed cookie. `/auth/google/callback` checks
`state`, exchanges the code with the client secret and the PKCE verifier, verifies the returned ID token
as above plus `nonce`, then signs in.

**Desktop.** Google refuses OAuth in embedded webviews, so the desktop app never renders Google's page.
The app's sign-in button calls a Tauri command that opens the system browser at
`/auth/google/start?client=desktop&dk=<desktop_key>`, where `desktop_key` is a random value the app
generates and keeps in memory. After the callback, the server creates the session, stores a one-time
code (hashed, 60 s expiry, bound to `desktop_key`) and redirects the browser to
`vtradingview://auth?code=<code>`. The app's deep-link handler passes the code to the window, which POSTs
`{code, desktop_key}` to `/auth/desktop/exchange` and receives the session cookie in the app's own webview
cookie store. A code that is reused, expired, or presented with the wrong `desktop_key` is refused. This
flow is specified and tested in Phase 3, although the app ships in Phase 6.

**Sign-in effect.** On every successful verification: upsert identity and user, set `last_login_at`,
create a session, set `__Host-vtv_session` (random 256-bit, `HttpOnly`, `Secure`, `SameSite=Lax`,
`Path=/`, 30-day sliding expiry), and return `{user, csrf_token}`. The database stores only the SHA-256 of
the session id and of the CSRF token.

**Requests.** Every non-GET to `/api/me/*`, `/auth/logout` and `/admin/*` must carry `X-CSRF-Token`
matching the session and an `Origin` equal to `PUBLIC_ORIGIN`. The websocket upgrade reads the same
cookie; a socket opened signed-out stays anonymous until it is reopened.

**Admin.** A user is admin when their stored email is in `ADMIN_EMAILS` and `email_verified` is true.
Admin routes return 404 to everyone else, including signed-out visitors.

### Saved state and sync

**Server side.** `user_state(user_id, key, value, version, updated_at)`. `GET /api/me/state` returns
every key with its version. `PUT /api/me/state/:key` with `If-Match: <version>` stores the value and
returns the new version; a stale version returns 409 with the current value and version. Limits: 1 MB per
value, 256 keys per user (Q7 may add a total cap).

**Which keys sync.** An allow-list of key patterns in `sync.js`: the six `chartHub*` keys plus the Vela
key patterns found by the Q5 inventory. Everything else (UI scratch state, the One Tap dismissal flag)
stays local. `chartHubDraft` (the unsaved Pine editor buffer) syncs but never raises a conflict notice:
last write wins for drafts.

**Client side.** `sync.js` is loaded first in `hub.html` and in each engine page inside the iframe. In
each document it wraps `Storage.prototype.setItem` and `removeItem` for `localStorage`; a write to an
allow-listed key is forwarded to the parent page with `postMessage` (from the iframe) or handled directly
(in the parent). Only the parent page talks to the server, so there is one writer per tab. The parent
debounces each key for 2 s, then PUTs it with the version it last saw. On 200 it stores the new version
in `localStorage` under `vtv:ver:<key>`; on 409 it shows the conflict notice (CUJ 5).

**Sign-in merge (decided: the server wins, the local value is kept as a backup).** On sign-in the parent
fetches all server keys, then for each allow-listed key:

| Server has it | Browser has it | Values equal | Result |
|---|---|---|---|
| no | yes | n/a | upload the browser value |
| yes | no | n/a | write the server value into `localStorage` |
| yes | yes | yes | nothing |
| yes | yes | no | write the server value into `localStorage`; upload the browser value as `backup:<key>:<YYYY-MM-DD>` |

When any backup was created, a notice says what was replaced and offers Restore per key, which writes
the backup back as the current value and uploads it with the current version. Backups expire after 30
days (deleted by a daily task) and count toward the key limit. If a server value replaced one the chart
is showing, the parent asks the iframe to reload once, after the merge, not once per key.

### MT5 bridges

**Bridge changes** (`bridge/mt5_bridge.py`): a required `BRIDGE_TOKEN`, checked as a bearer token on
every route with a constant-time compare; a `BRIDGE_BIND` option (still `127.0.0.1` by default; the user
publishes it over HTTPS with their own Cloudflare Tunnel or Caddy). No other behaviour changes.

**Server side.** `mt5_bridges` stores the URL, a label, and the token encrypted with XChaCha20-Poly1305
under `VTV_SECRET_KEY`. Every call goes through `BridgeClient`:

1.  The URL must be `https://`, with no user info, on port 443 or 1024 to 65535.
2.  Resolve the host; refuse if any resolved address is loopback, private (RFC 1918), CGNAT
    (100.64.0.0/10), link-local (169.254.0.0/16, fe80::/10), unique-local (fc00::/7), multicast,
    unspecified, or the IPv4-mapped form of any of these.
3.  Connect to the checked address itself (a custom resolver returns only the vetted addresses), so a
    second DNS answer cannot redirect the request. TLS still verifies the certificate for the host name.
4.  No redirects, 10 s total timeout, 5 MB response cap, at most 2 concurrent calls per bridge.

MT5 candles reach charts through `/api/bars?source=mt5&bridge=<id>` and a websocket subscription keyed
`(mt5:<bridge_id>, symbol, tf)`; both check that the bridge belongs to the session's user. Live updates
poll the bridge's `/wait` route, as the hub does today: one poller per bridge and symbol, shared only by
that user's open charts.

### Admin, logs and metrics

The log bus (`logbus.py`, `feedlog.py`) becomes a `tracing` layer that keeps the last 2,000 redacted lines
in a ring buffer and fans them out on an admin-only websocket channel. Redaction rules are ported as they
are and extended to session ids, CSRF tokens, bridge URL query strings and bridge tokens.
`/admin/restart` restarts a named feed task, not the process. The time-to-first-chart beacon
(`POST /api/metrics/ttfc`: milliseconds and engine name, no user id, rate-limited) feeds an in-memory
histogram that the admin panel reads from `GET /admin/metrics` (p50, p75, p95 and sample count per engine). It is lost on restart, which is acceptable for a weekly check.

### Deployment

`Procfile` lines, each run through `scripts/dev.sh` as in pricehub:

*   `db`: pgsqlite `--database data/vtv.db --port 55432` (dropped in production once `DATABASE_URL`
    points at managed Postgres).
*   `hub`: `hub-server` (release binary in production, `cargo run -p hub-server` in dev).
*   `tunnel`: `cloudflared tunnel run vtradingview` (production only).

`hub-server` refuses to start when `DATABASE_URL`, `GOOGLE_CLIENT_ID`, `VTV_SECRET_KEY` or
`PUBLIC_ORIGIN` is missing, and `/api/health` reports `ready: false` until migrations have run and the
database answers. Backups: a nightly SQLite online backup of `data/vtv.db` (pgsqlite keeps running), or
`pg_dump` on Postgres, kept 14 days off the host.

### Desktop shell

`desktop/tauri.conf.json` opens one window at `https://vtradingview.com/hub.html`. Capabilities: the
remote origin gets exactly two permissions, the custom command `start_sign_in` (opens the system browser
at the fixed start URL with the in-memory `desktop_key`) and the deep-link event. No file system, shell,
HTTP or other plugin permission is granted to the remote page. The `vtradingview://` scheme is registered
for the installed app. A tray icon offers Show and Quit. Navigation in the window is limited to
`https://vtradingview.com/*`; any other link, Google's sign-in included, opens in the system browser.

### New screens (direction `hub-native`)

All seven screens use only the hub's theme variables (`--bg`, `--panel`, `--raised`, `--line`, `--text`,
`--dim`, `--accent`, `--ok`, `--bad`, `--font`, `--mono`), its 3 to 6 px radii, 12 to 13 px UI text and
hairline separators. One amber element per view: the primary action. Contrast fixes from
[taste/hub-theme.md](taste/hub-theme.md), with ratios measured in this session from the preset values:

| Problem (measured) | Fix | Result (measured) |
|---|---|---|
| Light preset: `accentInk` on `accent` is 3.72 | `theme.js` derives `--onAccent`: whichever of `accentInk` and `text` contrasts more with `accent`; buttons use `--onAccent` | light 4.62; dark presets keep `accentInk` (8.34, 8.65) |
| Graphite `bad` text on panel is 4.02 | error messages use `--text`, with a `--bad` icon and the word "Error:" | text on panel 13.13; the icon is non-text and needs 3:1 |
| `line` as an input border is about 1.3 | input borders use `--dim` | 5.25 graphite, 5.70 black, 5.01 light |
| `faint` as text is 2.07 to 2.47 | placeholders and hints use `--dim` | 5.01 to 5.70 on panel |

Users can override any theme key in Settings, so `--onAccent` is computed at run time from their actual
colours rather than fixed per preset.

| Screen | Where it lives | Key states |
|---|---|---|
| Sign-in control and avatar menu | hub header, right side | signed out (button), signing in (busy), signed in (avatar; menu: sync status, MT5 bridges, delete account, sign out; admin: log panel) |
| Sync status and notices | a line in the avatar menu, plus a small notice under the header | saved, saving, offline (retry), conflict (CUJ 5 choices), backups created (Restore per key) |
| MT5 bridges | a new section in the existing Settings panel | empty, list, add form (URL, token, label), testing, test result (connected with symbol count, or the exact error), delete |
| Delete account dialog | modal from the avatar menu | confirm listing what is deleted; destructive styling with `--bad` on both trigger and confirm; busy; error |
| Admin log panel | the existing log panel, admin only | live log, per-feed health, TTFC p75, restart per feed with confirm |
| Desktop sign-in waiting | overlay in the desktop window | waiting, Try again, Cancel, failed (message says what to do) |
| Privacy page | `privacy.html`, static | one page in hub fonts and colours |

No native `<select>` in the new screens (repo rule); any picker uses the Listbox pattern. The gates in
`~/.agents/ux-ui-agent-skills/scripts/` (state contrast, responsive 280 to 951 px, keyboard, target size,
reduced motion) run on the D4 mockups and again on the built screens.

## Alternatives Considered

### Alternative A: Keep Python, bundle it as a Tauri sidecar

The first reference (`tauri-fastapi-full-stack-template`) ships the Python server as a PyInstaller
binary that Tauri starts, health-checks and kills.

*   **Pros**: no rewrite; the order-flow code stays in numpy/pandas; fastest route to a desktop app.
*   **Cons**: a second process per desktop install with its own start-up, health and Windows `taskkill`
    cleanup; a large binary (Python plus numpy, pandas, pyarrow); the server stays a threaded standard
    library HTTP server, which does not answer the many-users problem of a hosted site.
*   **Reason for Rejection**: the owner chose a hosted site with a thin desktop shell. A sidecar solves
    the desktop-install problem only, and the hosted site still needs a server built for many users.

### Alternative B: Axum embedded in the desktop app (the second reference's model)

`rust-full-stack-with-auth-template` runs a `backend` crate inside the Tauri process on a free port and
a separate `server` crate for the web.

*   **Pros**: the desktop app works without the hosted site; local MT5 could be reached directly.
*   **Cons**: two deployments of the server to keep in step; in the template they already drifted (axum
    0.5 in `backend`, 0.7 in `server`, different routes). A local server still could not hold synced
    accounts without the hosted database.
*   **Reason for Rejection**: the owner chose `tauri: embed https://vtradingview.com/hub.html`. Kept from
    the template: the workspace split and the shared types crate.

### Alternative C: Rewrite the frontend in Yew (or React)

*   **Pros**: one language across the stack (Yew); typed UI code.
*   **Cons**: re-implements about 20 vanilla JS modules, the Pine editor and converter, and the host
    pages around three chart engines; doubles the risk of a project whose server is already being
    replaced; the contract suite could no longer prove "nothing changed for the user".
*   **Reason for Rejection**: out of scope (Non-Goals). The frontend changes only where accounts and
    sync require it.

### Alternative D: JWT access tokens instead of server-side sessions

The second reference issues JWTs and authenticates websockets with a first-message handshake.

*   **Pros**: no session table lookup per request; stateless verification.
*   **Cons**: a JWT cannot be revoked before it expires, so sign-out, account deletion and a stolen token
    all wait for expiry unless a deny-list is added (which brings the database lookup back); tokens kept
    in JavaScript are exposed to any XSS; the first-message websocket handshake is extra protocol code.
*   **Reason for Rejection**: one server and one database make a session lookup cheap, and an `HttpOnly`
    cookie both revokes immediately and authenticates the websocket upgrade with no extra protocol.

### Alternative E: The sqlx SQLite driver directly, Postgres later

*   **Pros**: no pgsqlite process; SQLite is mature where pgsqlite is experimental.
*   **Cons**: two SQL dialects in the code (or sqlx's `Any` driver with its limits); the move to Postgres
    becomes a code change and a second test matrix, not a `DATABASE_URL` change.
*   **Reason for Rejection**: the owner chose pgsqlite so that one Postgres dialect is written once. The
    pgsqlite risk is handled by running every migration and query on both pgsqlite and Postgres 17 in
    CI, and switching production to Postgres the day pgsqlite misbehaves.

### Alternative F: MT5 without per-user bridge URLs

*   **(a) No MT5 for hosted users.** Simplest and removes the SSRF surface entirely, but drops a feature
    existing users rely on.
*   **(b) A local MT5 connector in the desktop app**, with `hub.html` reading `127.0.0.1` from the
    browser. Hits mixed-content and CORS rules in browsers, does not work on the web, and still needs the
    Python bridge on the user's machine.
*   **Reason for Rejection**: the owner chose (c), the server calling each user's own HTTPS bridge, which
    works on web and desktop alike. Its cost, the SSRF guard and bridge token, is specified in the
    Technical Plan and tested in Phase 7.

### Alternative G: Other merge rules at first sign-in

*   **Local value wins**: simplest, but signing in on an old or shared browser silently replaces newer
    account data, which is the normal case on a second machine.
*   **Ask per key**: the user decides, but Vela may hold dozens of keys, users cannot compare values in a
    list, and it turns the one-click sign-in of CUJ 2 into a dialog.
*   **Reason for Rejection**: the owner chose "server wins, keep a backup", which never loses either
    value and keeps sign-in to one click.

### Alternative H: Caddy with an open origin instead of a Cloudflare Tunnel

*   **Pros**: no dependency on `cloudflared`; standard TLS setup.
*   **Cons**: ports 80 and 443 open on the host; client IPs must be taken from proxy headers that anyone
    who reaches the origin directly could forge, unless the firewall admits only Cloudflare's ranges and
    that list is kept current.
*   **Reason for Rejection**: the owner chose a new Cloudflare Tunnel. With it the origin has no inbound
    ports, and `CF-Connecting-IP` can be trusted because only the tunnel reaches `127.0.0.1:8770`.

### Alternative I: A dealt design direction for the new screens

The design track normally deals three directions (`draw.mjs --options 3`) and the owner picks one from
rendered drafts.

*   **Pros**: tests whether a different look would serve the new screens better.
*   **Cons**: the new screens are a header control, a menu, a settings section and dialogs inside an
    existing product; a different look would make them read as foreign to the chart UI around them.
*   **Reason for Rejection**: the owner fixed the direction to the current hub theme (`hub-native`,
    recorded in `direction-approved.md`). No options were dealt, so there are no rejected directions to
    list; the alternative weighed is "a new direction" as a whole.


--------------------------------------------------------------------------------

## Non-Functional Requirements

*   **Security & Privacy**: as specified in the Technical Plan (sessions, CSRF, Google token checks,
    admin 404, SSRF guard, encrypted bridge tokens, CSP, closed origin). Personal data stored: email,
    name, avatar URL, Google `sub`. Account deletion is a hard delete with cascade. A privacy page ships
    before the public launch.
*   **Performance & Latency**: p75 time to first live chart under 2 s (G2). `/api/bars` p95 under 300 ms
    on a cache hit. The server must hold 1,000 concurrent websocket connections on the production host
    with the shared FeedHub; this is measured with a load script in Phase 4, not assumed.
*   **Rollout & Rollback**: the Python server stays runnable until the Phase 2 parity gate passes.
    Every migration has a down migration, tested in CI on both engines. A release is the `hub-server`
    binary plus the `hub/` folder; rollback is starting the previous binary under gaffa. Phase 4 starts
    with sign-in off (`AUTH_ENABLED=false`) until Q2 and Q3 are cleared, so public charts can be tested
    without accounts.

--------------------------------------------------------------------------------

### Additional Resources

*   [.paul/PLANNING.md](../PLANNING.md): the approved plan (phases, risks, dependencies).
*   [taste/hub-theme.md](taste/hub-theme.md): theme variables and measured contrast.
*   `~/project/tauri/rust-full-stack-with-auth-template`: workspace layout, shared types crate, Tauri
    `custom-protocol` feature.
*   `~/project/pricehub/Procfile`, `~/project/pricehub/scripts/dev.sh`: gaffa and pgsqlite service table.
*   `~/project/pgsqlite` (baysao fork, branch `fix/insert-select-returning-timestamptz`).

### Future Improvements

*   **Watchlists and alerts**: would add tables keyed by `user_id`; `user_state` is not the place.
*   **More than one server instance**: FeedHub fan-out moves to a shared bus (Redis or NATS); sessions
    are already in the database.
*   **Persistent metrics**: the TTFC histogram is in memory; export to Prometheus if weekly checks need
    history.
*   **Other sign-in providers**: `identities` already allows them.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Phases match `.paul/PLANNING.md`. Each file is marked create (C), modify (M) or delete (D). Paths are
from the repository root.

### Phase 1: Rust workspace and hub-server parity

#### 1. `Cargo.toml` (C)
*   **Type / Scope**: Cargo workspace manifest.
*   **Proposed Changes**: members `crates/hub-types`, `crates/hub-core`, `crates/hub-server`, and
    `crates/desktop` from Phase 6. `[workspace.dependencies]` pins one version each of axum 0.8, tokio 1,
    tower-http 0.6, tokio-tungstenite, reqwest (rustls), serde, serde_json, tracing, tracing-subscriber,
    so no crate can drift.

#### 2. `rust-toolchain.toml` (C)
*   **Type / Scope**: toolchain pin.
*   **Proposed Changes**: stable channel with `clippy` and `rustfmt`, so CI and dev build alike.

#### 3. `crates/hub-types/Cargo.toml`, `crates/hub-types/src/lib.rs` (C)
*   **Type / Scope**: Rust library, the API contract.
*   **Proposed Changes**: serde types for every response the Python server returns today: `Bar {time,
    open, high, low, close, volume}`, `StatusResponse` (per feed `{ok, ...}`), `SymbolsResponse`,
    `Instrument`, `HealthResponse`, and the websocket messages (subscribe, unsubscribe, bar, tick, error
    with a `kind` that includes `capacity`). Field names and optionality are copied from `hub/server.py`
    output, not redesigned.

#### 4. `crates/hub-core/Cargo.toml`, `crates/hub-core/src/lib.rs` (C)
*   **Type / Scope**: Rust library, the router.
*   **Proposed Changes**: `pub fn app(cfg: Config, state: AppState) -> Router`. `Config` comes from the
    environment (`HUB_HTTP_PORT`, `DATA_DIR`, `PUBLIC_ORIGIN`; later `DATABASE_URL` and auth keys).

#### 5. `crates/hub-core/src/static_files.rs` (C)
*   **Type / Scope**: static serving.
*   **Proposed Changes**: port of `static_allowed` in `hub/server.py`: `/` serves `hub.html`; only
    top-level files and `vendor/` files with extensions `.html .js .css .png .svg .ico .woff .woff2`; a
    segment starting with `.` or a backslash anywhere is refused; everything else is 404.

#### 6. `crates/hub-core/src/feeds/mod.rs`, `deriv.rs`, `bybit.rs`, `binance.rs`, `hub.rs` (C)
*   **Type / Scope**: upstream feed clients and FeedHub.
*   **Proposed Changes**: ports of the Deriv, Bybit and Binance paths in `hub/server.py` (connect,
    subscribe, reconnect with backoff, symbol lists). `hub.rs` implements the shared-subscription
    broadcast, the 60 s grace timer and the three limits (16 per connection, 4 connections per IP,
    `FEED_MAX_SYMBOLS` global).

#### 7. `crates/hub-core/src/routes/market.rs`, `ws.rs` (C)
*   **Type / Scope**: HTTP and websocket handlers.
*   **Proposed Changes**: `/api/status`, `/api/health`, `/api/symbols`, `/api/instruments`, `/api/bars`
    with the bars LRU; `/ws` upgrade joining FeedHub channels, with a 30 s ping for Cloudflare's idle
    timeout.

#### 8. `crates/hub-core/src/logbus.rs` (C)
*   **Type / Scope**: logging.
*   **Proposed Changes**: port of `hub/logbus.py` and `hub/feedlog.py`: a `tracing` layer, a 2,000-line
    ring buffer, the existing redaction rules. Served at `/api/logs` in Phase 1 for parity, moved to
    `/admin/logs` in Phase 3.

#### 9. `crates/hub-core/src/net.rs` (C)
*   **Type / Scope**: middleware.
*   **Proposed Changes**: client IP from `CF-Connecting-IP`, per-IP token-bucket rate limiter, security
    headers.

#### 10. `crates/hub-server/Cargo.toml`, `crates/hub-server/src/main.rs` (C)
*   **Type / Scope**: binary.
*   **Proposed Changes**: load `.env` (dotenvy), start tracing, build `AppState`, bind `127.0.0.1` on
    `HUB_HTTP_PORT` (default 8770), and print the start-up lines the Python server prints so the
    `AGENTS.md` checks still read correctly.

#### 11. `crates/hub-core/tests/contract.rs` (C)
*   **Type / Scope**: contract test suite.
*   **Proposed Changes**: runs against `CONTRACT_BASE_URL` (Python or Rust). Cases: every `AGENTS.md`
    check (`/api/status` feeds ok; `/api/bars?source=deriv&symbol=frxXAUUSD&tf=M1&limit=3` returns 3
    bars; `/.env` is 404); every response deserialises into `hub-types`; a `/ws` subscribe returns a bar
    within 15 s; path traversal and dotfile probes return 404.

#### 12. `hub/lab.js` (M)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: `wsUrl()` returns `/ws` on the page's own origin (`wss:` on https) instead of
    `ws://<host>:8765`; `window.LabWsPort` is removed. This is the only websocket URL in the frontend
    (`vela.bundle.js` has none).

#### 13. `Procfile`, `scripts/dev.sh` (C)
*   **Type / Scope**: process management.
*   **Proposed Changes**: `hub` line only in Phase 1. `dev.sh` follows pricehub's service-table shape
    (`svc_cmd`, `svc_port`, port check, `_svc` wrapper, and the note not to pass `gaffa --shell bash` on
    Windows).

#### 14. `.gitignore` (M)
*   **Type / Scope**: config.
*   **Proposed Changes**: add `target/`, `data/`, `vendor/pgsqlite/`. `.paul/` stays tracked.

#### 15. `.github/workflows/ci.yml` (C)
*   **Type / Scope**: CI.
*   **Proposed Changes**: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo audit`;
    a contract job that starts `python hub/server.py` and `hub-server` and runs the suite against each.

### Phase 2: Order flow in Rust

#### 16. `crates/hub-core/src/orderflow/mod.rs`, `trades.rs`, `archive.rs`, `delta.rs`, `footprint.rs`, `profile.rs`, `tpo.rs`, `estimate.rs` (C)
*   **Type / Scope**: computation.
*   **Proposed Changes**: ports of `hub/orderflow.py`, `orderflow_api.py` and `orderflow_trades.py`.
    `archive.rs` downloads and caches Binance trade archives under `DATA_DIR/orderflow/` in the Python
    layout. Routes `/api/orderflow/delta`, `/footprint`, `/profile` keep their query parameters.

#### 17. `crates/hub-core/tests/orderflow_golden.rs`, `crates/hub-core/tests/fixtures/orderflow/*.json` (C)
*   **Type / Scope**: parity tests.
*   **Proposed Changes**: fixtures captured from the Python code on fixed trade files (BTCUSDT and
    PAXGUSDT, one day each, plus one estimate-only symbol). Rust output must match within 1e-9 relative
    error per value.

#### 18. `scripts/capture_orderflow_fixtures.py` (C)
*   **Type / Scope**: one-off tool.
*   **Proposed Changes**: runs the Python order-flow functions on the fixture inputs and writes the JSON
    fixtures. Deleted in Phase 4 with the Python server.

### Phase 3: Database and auth

#### 19. `migrations/0001_users.up.sql`, `migrations/0001_users.down.sql` (C)
*   **Type / Scope**: schema.
*   **Proposed Changes**: `users`, `identities` (unique `(provider, subject)`), `sessions`,
    `desktop_auth_codes`. UUIDs generated in Rust, not by database functions, for pgsqlite. Timestamps
    as `timestamptz`.

#### 20. `crates/hub-core/src/db.rs` (C)
*   **Type / Scope**: data access.
*   **Proposed Changes**: `PgPool` from `DATABASE_URL`, `sqlx::migrate!`, readiness check for
    `/api/health`. Runtime-checked queries (`sqlx::query_as`), not compile-time macros, so builds do not
    need a live database.

#### 21. `crates/hub-core/src/auth/mod.rs`, `google.rs`, `session.rs`, `csrf.rs`, `desktop.rs`, `admin.rs` (C)
*   **Type / Scope**: authentication.
*   **Proposed Changes**: `google.rs`: JWKS cache and ID-token checks; authorization URL with state,
    nonce and PKCE; code exchange. `session.rs`: create, load (extractor `SessionUser`), slide, revoke.
    `csrf.rs`: token issue plus header and `Origin` check. `desktop.rs`: one-time code issue and exchange
    bound to `desktop_key`. `admin.rs`: `AdminUser` extractor that answers 404 on failure. For tests,
    the JWKS URL, issuer and token endpoint come from `Config` (`GOOGLE_JWKS_URL`, `GOOGLE_ISSUER`,
    `GOOGLE_TOKEN_URL`), and a `test-idp` cargo feature adds a local signer that `auth.rs` and
    `sync_harness.js` use; release builds are compiled without that feature and refuse a non-Google
    issuer.

#### 22. `crates/hub-core/src/routes/auth.rs`, `admin.rs`, `me.rs` (C)
*   **Type / Scope**: handlers.
*   **Proposed Changes**: `/auth/google/onetap`, `/auth/google/start`, `/auth/google/callback`,
    `/auth/desktop/exchange`, `/auth/me`, `/auth/logout`; `/admin/logs`, `/admin/restart`, `/admin/metrics`;
    `DELETE /api/me`. `/api/logs*` and `/api/restart` are removed.

#### 23. `crates/hub-core/tests/auth.rs`, `admin.rs`, `db_engines.rs` (C)
*   **Type / Scope**: integration tests.
*   **Proposed Changes**: forged tokens (wrong `aud`, expired, bad signature, wrong `nonce`) refused;
    first sign-in creates one user and the second reuses it; missing or wrong CSRF refused; desktop code
    reuse, expiry and wrong key refused; non-admin gets 404 on every `/admin/*`. `db_engines.rs` runs all
    migrations up and down. CI runs these against pgsqlite and against Postgres 17.

#### 24. `hub/auth.js` (C)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: loads Google Identity Services; One Tap with the `sessionStorage` dismissal
    flag; header button; `/auth/me` on load; sign-out; delete-account call; CSRF token kept in memory.
    Hidden entirely when `/auth/me` reports auth disabled.

#### 25. `hub/hub.html` (M)
*   **Type / Scope**: frontend host page.
*   **Proposed Changes**: loads `sync.js` first, then `auth.js`; the header gains the sign-in control and
    avatar menu; the remote-MT5 restart button (POST `/api/restart`, lines 195 to 196 today) is removed;
    the log panel entry shows only for admins.

#### 26. `hub/log_panel.js` (M)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: fetches `/admin/logs` instead of `/api/logs`, sends the CSRF header on the
    journal POST, adds per-feed health and the TTFC p75 line.

#### 27. `hub/theme.js` (M)
*   **Type / Scope**: frontend theme.
*   **Proposed Changes**: `HubThemeApply` computes `--onAccent` as whichever of `accentInk` and `text`
    contrasts more with `accent` (WCAG formula on the user's actual colours).

#### 28. `.env.example` (M)
*   **Type / Scope**: config template.
*   **Proposed Changes**: add `DATABASE_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
    `VTV_SECRET_KEY`, `ADMIN_EMAILS`, `PUBLIC_ORIGIN`, `AUTH_ENABLED`, `FEED_MAX_SYMBOLS`,
    `PGSQLITE_PORT`; remove `HUB_WS_PORT` and the remote-MT5 SSH settings.

#### 29. `Procfile`, `scripts/dev.sh` (M)
*   **Type / Scope**: process management.
*   **Proposed Changes**: add the `db` line (pgsqlite built from `~/project/pgsqlite` or
    `vendor/pgsqlite`, `--database data/vtv.db --port 55432`).

### Phase 4: Public deploy

#### 30. `Procfile`, `scripts/dev.sh` (M)
*   **Type / Scope**: process management.
*   **Proposed Changes**: add the `tunnel` line (`cloudflared tunnel run vtradingview`, production only).

#### 31. `ops/cloudflared/config.yml.example`, `ops/backup.sh`, `ops/README.md` (C)
*   **Type / Scope**: operations.
*   **Proposed Changes**: tunnel ingress from `vtradingview.com` to `http://127.0.0.1:8770` with a 404
    catch-all; nightly SQLite online backup or `pg_dump`, 14-day retention, off-host copy; restore steps
    and the restore drill record.

#### 32. `crates/hub-core/src/routes/metrics.rs`, `hub/ttfc.js` (C)
*   **Type / Scope**: metrics.
*   **Proposed Changes**: `/api/metrics/ttfc` histogram and its admin read at `/admin/metrics`; the page measures navigation start to the first
    live candle and posts once per load.

#### 33. `hub/privacy.html`, `hub/notices.html` (C)
*   **Type / Scope**: static pages.
*   **Proposed Changes**: privacy page; licence attribution for Lightweight Charts, KLineChart and Vela
    (from `licenses/` and `THIRD_PARTY_NOTICES.md`), linked from the hub.

#### 34. `scripts/load_ws.mjs` (C)
*   **Type / Scope**: load test.
*   **Proposed Changes**: opens N websocket connections with realistic subscriptions and reports
    connections held, memory and upstream subscription count.

#### 35. `hub/server.py`, `hub/orderflow.py`, `hub/orderflow_api.py`, `hub/orderflow_trades.py`, `hub/logbus.py`, `hub/feedlog.py`, `scripts/capture_orderflow_fixtures.py`, `start.bat`, `start.sh` (D); `requirements.txt` (M)
*   **Type / Scope**: retired Python server.
*   **Proposed Changes**: deleted on this branch after the Phase 2 parity gate and the Phase 4 smoke
    test. `requirements.txt` keeps only what `bridge/mt5_bridge.py` needs. The contract job in `ci.yml`
    then runs against Rust only.

#### 36. `README.md`, `AGENTS.md`, `SECURITY.md` (M)
*   **Type / Scope**: documentation.
*   **Proposed Changes**: hosted-site usage, `cargo` and gaffa dev setup, the updated agent checks (one
    port, `/admin/*`), and the new threat model (public origin behind a tunnel, sessions, SSRF).

### Phase 5: Synced state

#### 37. `migrations/0002_user_state.up.sql`, `migrations/0002_user_state.down.sql` (C)
*   **Type / Scope**: schema.
*   **Proposed Changes**: `user_state(user_id, key, value text, version integer, updated_at)`, primary key
    `(user_id, key)`, cascade on user delete. The value is `text` holding JSON on both engines, because
    the server never queries inside it.

#### 38. `crates/hub-core/src/routes/state.rs`, `crates/hub-core/src/jobs.rs` (C)
*   **Type / Scope**: handlers and background task.
*   **Proposed Changes**: `GET /api/me/state`, `PUT /api/me/state/:key` with `If-Match` and 409, size and
    key-count limits; a daily job deleting `backup:*` keys older than 30 days.

#### 39. `crates/hub-core/tests/state.rs` (C)
*   **Type / Scope**: integration tests.
*   **Proposed Changes**: cross-user reads and writes refused (G5); a stale version gets 409 with the
    current value; limits enforced; backups expire.

#### 40. `hub/sync.js` (C)
*   **Type / Scope**: frontend sync layer.
*   **Proposed Changes**: `Storage.prototype.setItem` and `removeItem` wrappers; key allow-list;
    iframe-to-parent `postMessage` forwarding checked against `location.origin`; 2 s debounce per key;
    versions stored under `vtv:ver:<key>`; sign-in merge per the rule table; conflict and backup notices;
    one iframe reload after the merge.

#### 41. `hub/vela.html`, `hub/lightweight.html`, `hub/klinechart.html`, `hub/index.html` (M)
*   **Type / Scope**: engine pages.
*   **Proposed Changes**: load `sync.js` as the first script.

#### 42. `docs/vela-storage-keys.md` (C)
*   **Type / Scope**: inventory (answers Q5).
*   **Proposed Changes**: the real key names Vela writes, captured by running the app with a logging
    `setItem` wrapper, and the patterns added to the allow-list.

#### 43. `hub/tests/sync_harness.js` (C)
*   **Type / Scope**: Playwright test.
*   **Proposed Changes**: CUJ 3, 4 and 5 end to end against a local `hub-server` with a test identity
    provider: draw signed-out, sign in, see the drawing in a second browser context; a concurrent edit
    shows the conflict notice; clashing keys produce a backup and Restore works.

### Phase 6: Tauri desktop

#### 44. `crates/desktop/Cargo.toml`, `crates/desktop/build.rs`, `crates/desktop/src/main.rs`, `crates/desktop/src/lib.rs` (C)
*   **Type / Scope**: Tauri 2 app.
*   **Proposed Changes**: window at `PUBLIC_ORIGIN/hub.html`; `start_sign_in` command (generates the
    `desktop_key`, opens the system browser with the opener plugin); deep-link handler forwarding the
    code to the window; navigation guard keeping the window on `vtradingview.com`; tray with Show and
    Quit.

#### 45. `crates/desktop/tauri.conf.json`, `crates/desktop/capabilities/remote.json`, `crates/desktop/icons/*` (C)
*   **Type / Scope**: Tauri config.
*   **Proposed Changes**: product name, identifier, deep-link scheme `vtradingview`, the
    `custom-protocol` feature; a capability file granting the remote origin only `start_sign_in` and the
    deep-link event.

#### 46. `hub/auth.js` (M)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: when `window.__TAURI__` is present, the sign-in button calls `start_sign_in`
    and shows the waiting overlay; on the deep-link event it POSTs to `/auth/desktop/exchange`.

#### 47. `.github/workflows/desktop.yml` (C)
*   **Type / Scope**: CI.
*   **Proposed Changes**: on tags, build installers for Windows, macOS and Linux; sign them when
    certificates are configured (Q4), otherwise publish unsigned betas.

### Phase 7: Per-user MT5 bridges

#### 48. `bridge/mt5_bridge.py` (M)
*   **Type / Scope**: Python bridge.
*   **Proposed Changes**: a required `BRIDGE_TOKEN` bearer check on every route
    (`hmac.compare_digest`), a `BRIDGE_BIND` option, and a refusal to start without a token. Read-only
    behaviour is unchanged.

#### 49. `migrations/0003_mt5_bridges.up.sql`, `migrations/0003_mt5_bridges.down.sql` (C)
*   **Type / Scope**: schema.
*   **Proposed Changes**: `mt5_bridges(id, user_id, label, url, token_ciphertext, token_nonce,
    last_ok_at, last_error, created_at)`, cascade on user delete.

#### 50. `crates/hub-core/src/mt5/mod.rs`, `guard.rs`, `client.rs`, `crypto.rs`, and `crates/hub-core/src/routes/mt5.rs` (C)
*   **Type / Scope**: bridge client and SSRF guard.
*   **Proposed Changes**: `guard.rs`: URL and address checks with the vetted-address resolver.
    `client.rs`: no redirects, 10 s timeout, 5 MB cap, 2 concurrent calls per bridge. `crypto.rs`:
    XChaCha20-Poly1305 under `VTV_SECRET_KEY`. Routes `/api/me/mt5*`, and `source=mt5` on bars and `/ws`.

#### 51. `crates/hub-core/tests/ssrf.rs` (C)
*   **Type / Scope**: security tests.
*   **Proposed Changes**: refused: `http://` URLs; user info in the URL; loopback v4 and v6; RFC 1918;
    `169.254.169.254`; CGNAT; `fc00::/7`; IPv4-mapped IPv6; a host that resolves to a public address
    first and a private one second (rebinding); a redirect to a private address. Owner-only access.

#### 52. `hub/settings_mt5.js` (C), `hub/hub.html` (M)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: the MT5 bridges section of the Settings panel (empty, list, add, test, result,
    delete); MT5 bridges appear as sources in the symbol picker.

#### 53. `docs/mt5-bridge-hosted.md` (C)
*   **Type / Scope**: user documentation.
*   **Proposed Changes**: running the bridge with a token and publishing it over HTTPS with Cloudflare
    Tunnel or Caddy.

### Verification Plan

*   **Compilation/Build**: `cargo build --workspace --release`; `cargo tauri build` in `crates/desktop`
    (Phase 6).
*   **Unit Tests**: `cargo test --workspace`.
*   **Static Analysis & Linting**: `cargo fmt --check`, `cargo clippy --workspace -D warnings`,
    `cargo audit`.
*   **Contract**: `contract.rs` against Python and Rust in Phase 1, Rust only after Phase 4; green in
    every later phase.
*   **Parity**: `orderflow_golden.rs` within 1e-9 relative error, from Phase 2 on.
*   **Database engines**: `db_engines.rs`, `auth.rs` and `state.rs` on pgsqlite and Postgres 17 in CI.
*   **Security**: `auth.rs`, `admin.rs`, the cross-user cases in `state.rs`, and `ssrf.rs`; zero failures
    allowed (G5, G6).
*   **E2E/Integration**: `hub/tests/sync_harness.js` (Playwright) for CUJ 3 to 5; a desktop sign-in
    round trip on each OS (manual checklist in Phase 6).
*   **UI gates** on each new screen, from `~/.agents/ux-ui-agent-skills/scripts/`:
    `accuracy_report.mjs` (with `DS_REQUIRE_BROWSER=1`), `verify_states.mjs`, `verify_responsive.mjs`,
    `verify_keyboard.mjs`, `verify_target_size.mjs`, `verify_reduced_motion.mjs`, in light and dark
    presets.
*   **Performance**: TTFC p75 from the beacon under 2 s over one week of real traffic; `load_ws.mjs`
    holding 1,000 connections (Phase 4).
*   **Test Coverage Requirement**: 80% line coverage on `hub-core` (`cargo llvm-cov`); every branch of
    `auth/`, `mt5/guard.rs` and `routes/state.rs` covered by a named test.

--------------------------------------------------------------------------------

<!-- Provenance: Indicates initial structural drafting via EGM. -->

*Drafted with EGM*
