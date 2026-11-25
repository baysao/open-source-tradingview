# vtradingview: hosted TradeX on Rust, with accounts and a desktop shell

*   **Author:** Vu Tran
*   **Status:** Proposed
*   **Date:** 2026-11-23
*   **Brief:** [.paul/PLANNING.md](../../PLANNING.md)
*   **Design direction:** `hub-native`, seed `none` (fixed by the owner, no draw). See
    [direction-approved.md](../directions/vtradingview/direction-approved.md) and
    [taste/hub-theme.md](../taste/hub-theme.md).

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
    Exceptions (D-001): `/api/symbols` drops `mt5` and `arm`, `ws_port` reports 8770 until `hub/lab.js`
    derives the websocket URL, and `/api/restart` is not ported.
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
*   **Porting Local MT5 and SSH Remote MT5** (`LOCAL_MT5`, source `arm`). Decision D-001
    (`.paul/DECISIONS.md`): local mode (`AUTH_ENABLED=false`, no database, `127.0.0.1` only) serves charts
    and the free feeds; MT5 returns in Phase 7 through per-user bridges, with loopback bridge URLs allowed
    only when auth is off. The last Python release is tagged `python-final`.

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
remote origin gets exactly one permission, the custom command `start_sign_in` (opens the system browser
at the fixed start URL with the in-memory `desktop_key`); the deep-link event arrives by a Rust-side
`eval` of a `CustomEvent`, needing no permission (amended in Phase 06, D-029/D-030). No file system, shell,
HTTP or other plugin permission is granted to the remote page. The `vtradingview://` scheme is registered
for the installed app. A tray icon offers Show and Quit. Navigation in the window is limited to
`https://vtradingview.com/*`; any other link, Google's sign-in included, opens in the system browser.

### New screens (direction `hub-native`)

All seven screens use only the hub's theme variables (`--bg`, `--panel`, `--raised`, `--line`, `--text`,
`--dim`, `--accent`, `--ok`, `--bad`, `--font`, `--mono`), its 3 to 6 px radii and hairline separators.
Text in the new screens is 12 px for short labels and 14 px for everything else, one step above the
hub's 11 to 13 px chart chrome; the existing chart UI keeps its sizes. (Set at D4: the lint's
readability floor, applied once in the mockups' shared stylesheet, see
[polish-log.md](../polish-log.md).) One amber element per view: the primary action. Contrast fixes from
[taste/hub-theme.md](../taste/hub-theme.md), with ratios measured in this session from the preset values:

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

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| pgsqlite (experimental) mishandles a query or migration | med | high | CI runs every migration and query on pgsqlite and Postgres 17; pools verify each connection before use (D-018); production switches `DATABASE_URL` to Postgres the day it misbehaves |
| Upstream feeds throttle or ban the single server IP | med | high | One shared upstream per (source, symbol), the bars cache, four per-IP rate buckets, the admin feed-health view |
| Binance or Bybit blocks the hosting region | med | high | Choose a non-US region; verify endpoints from the host before Phase 4 sign-off (DEBT-018) |
| Deriv, Bybit and Binance data redistribution terms forbid public serving (Q2) | med | high | Read each provider's API terms before launch; drop or gate a feed that forbids it |
| The "vtradingview" name conflicts with the TradingView trademark (Q3) | med | high | Legal review before public launch; the non-affiliation notice ships on `/privacy.html`; an alternate name held in reserve |
| Google blocks sign-in inside the Tauri webview | high (by policy) | high | The system-browser + deep-link flow is designed in from the start (Phase 6) |
| Order-flow numbers drift from the Python reference | low (closed in Phase 2) | med | Golden fixtures with an explicit tolerance gate shipped in Phase 2; the Python reference is retired |
| SSRF through user bridge URLs (Phase 7) | med | high | Resolve-and-pin guard, deny private ranges, no redirects, size and time caps, a dedicated test suite |
| The Tauri remote page gains IPC access (Phase 6) | low | high | Capabilities grant nothing to the remote origin beyond opener and deep link; audited in Phase 6 |
| Licence attribution missing on the hosted site | low (shipped in Phase 4) | med | `/notices.html` carries the component table, the served licence texts and the three attribution requirements |
| The Pine compiler needs `script-src 'unsafe-eval'` | certain (named residual) | med | `vela.bundle.js` compiles Pine through `new Function`; the hash-locked CSP still blocks every rogue inline script. Removing the residual means a Vela change; named here so it stays a decision, not a surprise |
| One host with nightly backups: up to a day of synced state is lost on host loss | low | med | Backups are nightly and off-host, kept 14 days; the restore drill is rehearsed (`ops/restore_drill.sh`); a shorter window is a config change once state sync has real users |

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

## Additional Resources

*   [.paul/PLANNING.md](../../PLANNING.md): the approved plan (phases, risks, dependencies).
*   [taste/hub-theme.md](../taste/hub-theme.md): theme variables and measured contrast.
*   `~/project/tauri/rust-full-stack-with-auth-template`: workspace layout, shared types crate, Tauri
    `custom-protocol` feature.
*   `~/project/pricehub/Procfile`, `~/project/pricehub/scripts/dev.sh`: gaffa and pgsqlite service table.
*   `~/project/pgsqlite` (baysao fork, branch `fix/insert-select-returning-timestamptz`).

## Future Improvements

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

Phase 1 facts, measured in the code on 2026-11-23 (they replace the earlier draft of this phase):

*   The Python server serves `hub/` as its static root on `HUB_HTTP_PORT` (8770) and the websocket on
    `HUB_WS_PORT` (8765), both bound to `127.0.0.1` (`hub/server.py:1050-1051`, `1065`).
*   `/ws` protocol: one subscription per socket, untyped `{source, symbol, tf}`, a new message replaces the
    stream (`hub/server.py:687-704`); replies `history`, `bar`, `status`, `error` (D-002).
*   `/api/bars` takes `source`, `symbol`, `tf`, `limit`, `to`; an unknown `source` falls back to Binance;
    errors return 502 `{"error": "<text>"}` (`hub/server.py:976-990`; client `hub/lab.js:31`).
*   `/api/status` returns `{deriv, bybit, binance}` each `{ok, msg}`, cached 5 s, plus `arm` and `mt5` when
    enabled (`hub/server.py:865-882`). `/api/health` returns `{clock_offset_ms, feeds: {"<source>:<symbol>":
    {ticks, age_ms, lag_median_ms, lag_p95_ms}}}` from an SNTP query to `time.windows.com` and the last 300
    lags per feed (`hub/server.py:515-552`).
*   Log bus: ring of 12,000 lines (`hub/logbus.py:17`), 2,000-character message cap, rotating
    `hub/logs/hub.log` (2 MB x 3) reloaded at boot, redaction before storage, source silence checks and
    `PROBLEMS`. `snapshot` returns `{boot, last, now, errors_15m, latest_error, problems, sources}` plus
    `lines` (limit 3,000) unless `brief` (`hub/logbus.py:243-256`). Feedlog counts per provider and
    summarises every 60 s, and marks a stream silent after 120 s (`hub/feedlog.py:9-13`).
*   POST rules today: only `/api/logs/journal` and `/api/restart`; `Origin` must be
    `http://localhost:<port>` or `http://127.0.0.1:<port>` (403 otherwise), JSON only (415), body cap
    300,000 bytes (413) (`hub/server.py:989-1013`).
*   Static: `/` serves `hub/index.html` (a meta refresh to `hub.html`); paths are percent-decoded before
    the checks; static responses carry `Cache-Control: no-cache, must-revalidate`, API responses
    `no-store`; a refused path returns 404 `{"error": "not found"}` (`hub/server.py:888-898`, `971-973`,
    `1018-1034`).
*   Start-up lines today: `Clock offset vs time.windows.com: <+N> ms (corrected in lag numbers)` and
    `TradeX: open http://localhost:8770/hub.html  (data on ws://localhost:8765)` (`hub/server.py:1060`,
    `1064`), plus `Bybit: <n> charts loaded` and `Deriv: <n> symbols loaded` (`AGENTS.md:27-31`).
*   The Restart button POSTs `/api/restart` only when the broker is `arm`; for every other feed it reloads
    the chart (`hub/hub.html:190-201`).

Decisions applied: D-001 (local mode, no MT5 ports) and D-002 (protocol kept, upstream per symbol).

#### 1. `Cargo.toml`, `Cargo.lock` (C)
*   **Type / Scope**: Cargo workspace manifest.
*   **Proposed Changes**: members `crates/hub-types`, `crates/hub-core`, `crates/hub-server`
    (`crates/desktop` is added by Phase 6, item 44). `[workspace.dependencies]` declares one version of
    each crate used by more than one member: axum 0.8 (feature `ws`), tokio 1 (`full`), tower-http 0.6
    (`fs`, `trace`, `set-header`), tokio-tungstenite (rustls), reqwest (rustls, json), serde and
    serde_json, tracing and tracing-subscriber, dotenvy, lru, regex, and `zip` (Phase 2, deflate only), at versions compatible with
    axum 0.8 (D-005: tower-http 0.6, reqwest 0.12, tokio-tungstenite 0.26, lru 0.12); `Cargo.lock` is
    committed and is the pin.

#### 2. `rust-toolchain.toml` (C)
*   **Type / Scope**: toolchain pin.
*   **Proposed Changes**: the stable release current on the day the plan is applied, written as an exact
    version, with `clippy` and `rustfmt`, so CI and local builds use the same compiler.

#### 3. `crates/hub-types/Cargo.toml`, `crates/hub-types/src/lib.rs` (C)
*   **Type / Scope**: Rust library, the Rust model of the contract (not the oracle: see item 11).
*   **Proposed Changes**: serde types for the responses in the Phase 1 facts: `Bar {time, open, high, low,
    close, volume}`, `FeedStatus {ok, msg}` and `StatusResponse` (feed name to `FeedStatus`),
    `SymbolsResponse` (`symbols`, `mt5: "off"`, `arm: "off"`, `deriv`, `bybit`, `ws_port`; D-003), `Instrument`,
    `HealthResponse {clock_offset_ms, feeds}`, `LogSnapshot`, and the websocket messages: client
    `Subscribe {source, symbol, tf}` (untyped, as today), server `History {bars}`,
    `BarMsg {bar, sent, trade_ms?, lag_ms?}`, `Status {msg}`, `Error {msg, kind?}`, tagged by `type`.
    Types use `#[serde(deny_unknown_fields)]` and no `Option` where Python always sends the field.

#### 4. `crates/hub-core/Cargo.toml`, `crates/hub-core/src/lib.rs`, `crates/hub-core/src/config.rs` (C)
*   **Type / Scope**: Rust library, the router.
*   **Proposed Changes**: one entry point, `pub fn app(state: AppState) -> Router`, where
    `AppState { cfg: Arc<Config>, feeds: FeedHub, bars: BarsCache, logs: LogBus, limiter: Limiter }`
    (Phase 3 adds `db`). `Config::from_env()` reads `HUB_HTTP_PORT` (default 8770), `HUB_STATIC_DIR`
    (default `hub`), `HUB_LOG_DIR` (default `hub/logs`), `PUBLIC_ORIGIN` (default
    `http://127.0.0.1:<port>`), `AUTH_ENABLED` (default `false`; only `false` exists in Phase 1),
    `FEED_MAX_SYMBOLS` (default 400). With `AUTH_ENABLED=false`, start-up fails unless the bind address is
    `127.0.0.1` (D-001).

#### 5. `crates/hub-core/src/static_files.rs` (C)
*   **Type / Scope**: static serving.
*   **Proposed Changes**: the `static_allowed` rule: percent-decode first; `/` serves
    `HUB_STATIC_DIR/index.html`; allow only top-level files and files directly under `vendor/` with
    extensions `.html .js .css .png .svg .ico .woff .woff2`; refuse any segment starting with `.` and any
    backslash; everything else is 404 `{"error": "not found"}`. Static responses carry
    `Cache-Control: no-cache, must-revalidate`.

#### 6. `crates/hub-core/src/feeds/mod.rs`, `deriv.rs`, `bybit.rs`, `binance.rs`, `hub.rs` (C)
*   **Type / Scope**: upstream feed clients and FeedHub.
*   **Proposed Changes**: ports of the Deriv, Bybit and Binance paths in `hub/server.py` (symbol lists,
    history, live streams). `hub.rs`: one upstream task per `(source, symbol)`, a bar builder per
    timeframe fanning out over `tokio::sync::broadcast`; the first viewer opens the upstream, the last one
    leaving starts a 60 s grace timer. Limits: 24 sockets per client IP; `FEED_MAX_SYMBOLS` distinct
    `(source, symbol)` upstreams, refused with `error` `{msg, kind: "capacity"}`. Reconnect: exponential
    backoff from 1 s to 60 s with full jitter for every provider (Python used fixed sleeps: Deriv 60 s,
    Bybit 2 s, Binance none). Every tick calls `record_tick` for `/api/health`.

#### 7. `crates/hub-core/src/routes/market.rs`, `ws.rs`, `health.rs`, `logs.rs` (C)
*   **Type / Scope**: HTTP and websocket handlers.
*   **Proposed Changes**: `/api/status` (5 s cache); `/api/health` (clock offset from one SNTP query to
    `time.windows.com:123` at start, last 300 lags per feed); `/api/symbols`; `/api/instruments`;
    `/api/bars` (unknown `source` falls back to Binance; 502 `{"error"}` on upstream failure) behind
    `BarsCache`, an LRU of 512 entries keyed `(source, symbol, tf, limit, to)` with a 5 s TTL when `to` is
    absent and 10 min when present. `/api/logs?since=&brief` and `POST /api/logs/journal` (same Origin,
    JSON-only and 300,000-byte rules, with `PUBLIC_ORIGIN` added to the allowed origins). `/ws`: the D-002
    protocol, a websocket ping frame every 30 s, the socket closed after 60 s without a pong.
    `/api/restart` is not ported (D-001). Every API response carries `Cache-Control: no-store`.

#### 8. `crates/hub-core/src/logbus.rs`, `crates/hub-core/src/feedlog.rs` (C)
*   **Type / Scope**: logging.
*   **Proposed Changes**: a `tracing` layer feeding a 12,000-line ring with the 2,000-character message
    cap; the redaction patterns of `hub/logbus.py` ported one to one; rotating `HUB_LOG_DIR/hub.log`
    (2 MB x 3) reloaded at boot; source silence checks and `problems`; the `snapshot` shape above.
    Feedlog: per-provider counters, 60 s summary, 120 s stream-silent rule. Not ported in Phase 1:
    `HUB_TAIL_FILES` tailing of local files, and the stdout `Tee` (the server's own output already goes
    through `tracing`); the arm link-port check goes with arm (D-001).

#### 9. `crates/hub-core/src/net.rs` (C)
*   **Type / Scope**: middleware.
*   **Proposed Changes**: client IP from `CF-Connecting-IP` only when the TCP peer is loopback (the tunnel),
    else the peer address; `tower_governor` limits per client IP on `/api/*`: 20 requests per second,
    burst 40, answering 429 with `Retry-After`; security headers `X-Content-Type-Options: nosniff` and
    `Referrer-Policy: same-origin`. The CSP is set in Phase 4, not here, because the current pages use
    inline scripts.

#### 10. `crates/hub-server/Cargo.toml`, `crates/hub-server/src/main.rs` (C)
*   **Type / Scope**: binary.
*   **Proposed Changes**: load `./.env` then `hub/.env` with dotenvy (real environment variables win, as in
    Python), start tracing, build `AppState`, bind `127.0.0.1:<HUB_HTTP_PORT>`. Start-up lines:
    `Clock offset vs time.windows.com: <+N> ms (corrected in lag numbers)`,
    `TradeX: open http://localhost:8770/hub.html  (data on ws://localhost:8770/ws)`,
    `Bybit: <n> charts loaded`, `Deriv: <n> symbols loaded`.

#### 11. `crates/hub-core/tests/contract.rs`, `crates/hub-core/tests/contract/requests.toml` (C)
*   **Type / Scope**: contract test suite (the G1 oracle).
*   **Proposed Changes**: for each request in `requests.toml` (every route in the Phase 1 facts, with fixed
    parameters), call both servers and compare the raw JSON: same key set at every level and the same JSON
    type per key; values ignored except where a case names them (`/.env` is 404 on both; bars count equals
    `limit`). Allowed differences are listed once in that file (D-001: `mt5` and `arm` absent from Rust;
    `ws_port` 8765 against 8770). For `/ws`, subscribe `binance BTCUSDT M1` and compare the set of `type`
    values and the field set of each within 20 s. Probes on Rust: `%2e%2e/`, `%5c`, `.env`,
    `vendor/a/b.js` and `server.py` all return 404. Env: `CONTRACT_PY_URL` and `CONTRACT_RS_URL`; with
    either unset the suite is skipped with a printed reason, never passed silently. Upstream rule: a case
    whose upstream fails on both servers with the same error is reported `skipped (upstream)`, not passed;
    a phase exit needs one full local run with zero skips.

#### 12. `hub/lab.js` (M)
*   **Type / Scope**: frontend.
*   **Proposed Changes**: `wsUrl()` returns `/ws` on the page's own origin (`wss:` on https) instead of
    `ws://<host>:<LabWsPort>`, built with the URL API. Line 95 keeps reading `ws_port`, `mt5` and `arm`
    (D-003, which amends D-001). Nothing else changes in the websocket code (D-002).

#### 12a. `hub/hub.html` (M)
*   **Type / Scope**: frontend host page.
*   **Proposed Changes**: `restartChart()` drops its `st.broker === "arm"` branch; the button keeps
    reloading the chart for every feed. The `arm` and `mt5` broker entries are removed from the broker
    list (D-001).

#### 13. `Procfile`, `scripts/dev.sh` (C)
*   **Type / Scope**: process management.
*   **Proposed Changes**: `Procfile` line `hub: bash scripts/dev.sh _svc hub`. `scripts/dev.sh` is written
    in this repository: `dev [svc...]` starts gaffa on the Procfile; `_svc hub` builds with
    `cargo build -p hub-server` and runs `target/debug/hub-server`; it refuses to start when port 8770 is
    taken. Windows note: do not pass `gaffa --shell bash`. gaffa is the Procfile process supervisor
    (https://github.com/oryon-dominik/interstellar-gaffa).

#### 14. `.gitignore` (M)
*   **Type / Scope**: config.
*   **Proposed Changes**: add `target/`, `data/`, `vendor/pgsqlite/`. `.paul/` is committed with the first
    Phase 1 commit.

#### 14a. `AGENTS.md` (M)
*   **Type / Scope**: agent setup guide.
*   **Proposed Changes**: section 1 gains the `cargo run -p hub-server` path beside Python, the new
    start-up line (one port, `/ws`) and the same three checks; the MT5 sections point to Phase 7 bridges
    and the `python-final` tag (D-001).

#### 15. `.github/workflows/ci.yml` (C)
*   **Type / Scope**: CI.
*   **Proposed Changes**: jobs on Ubuntu: `cargo fmt --check`;
    `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo audit`
    (installed with `taiki-e/install-action`); `contract`: Python 3.12 with
    `pip install -r requirements.txt`, starts `python hub/server.py` (8770) and `target/debug/hub-server`
    (`HUB_HTTP_PORT=8771`), sets `CONTRACT_PY_URL` and `CONTRACT_RS_URL`, runs
    `cargo test -p hub-core --test contract`. Skipped upstream cases are printed in the job summary.

### Phase 2: Order flow in Rust

**Phase 2 facts (measured in `hub/orderflow*.py` and `hub/server.py:921-968`, 2026-11-23).** Gate cleared 2026-11-23 (R5, R6, R10, R32).
*   Routes, all GET, all answering 502 `{"error": "<Class>: <msg>"}` on any failure and logging a warn
    line with `every=30`: `/api/orderflow/delta` (`symbol` default `frxXAUUSD`, `tf` default `M5`, `from`
    default now minus 1 day, `to` default now, `source` auto|real|estimated, `feed` picks the history
    function, default binance) returns `{"rows": [...], source, real_symbol, intrabar, unit, method}`;
    `/api/orderflow/footprint` (`from` default now minus 3 h, plus `row_size`, `fill`) returns
    `{"candles": [...], source, real_symbol, row_size, unit, covered_from, pending, est_candles, method}`;
    `/api/orderflow/profile` (plus `mode` sessions|day|week, `rows` 24, `row_size`, `va` 70, `developing`)
    returns `{"sessions": [...], source, real_symbol, intrabar, unit, mode, va_pct, method, shift}`.
    Callers: `hub/orderflow_delta.js`, `orderflow_profile.js`, `orderflow_svp_type.js`,
    `orderflow_charttypes.js`.
*   TPO is computed in the browser (`hub/orderflow_tpo_type.js`, from `/api/bars`); there is no TPO route
    and no TPO port (R6).
*   Source "real" is Binance USD-M futures (`fapi.binance.com/fapi/v1`): `real_symbol()` strips `RMT_`,
    `BYBIT_S_`, `BYBIT_I_`, `BYBIT_`, `FRX`, `CRY`, refuses `.MARK`/`.INDEX`, maps `USD247`, renames
    `XCU`->`COPPER`, `XNG`->`NATGAS`, and keeps `<BASE>USDT` only if `exchangeInfo` lists it as TRADING
    (refreshed every 6 h, retry after 5 min on failure). Gold maps to `XAUUSDT`, not `PAXGUSDT` (R32).
*   Constants (R10): `INTRABAR` M1/M5/M15->M1, M30/H1->M5, H4->M15, D1->H1; `MAX_ROWS = 20000` lower bars
    per request; klines pages of 1500 (weight 10); aggTrades pages of 1000 (weight 20); one weight budget
    for every Binance call: 1900 per minute of the 2400 allowed, background loading leaves a 400 reserve,
    a reply's `X-MBX-USED-WEIGHT-1M` caps at 1800 (background) or 2200 (chart), and HTTP 429 or 418 stops
    every call for `Retry-After` seconds (60 when absent); `MAX_PAST_DAYS = 2` daily files;
    `MAX_REST_HOURS = 30`; head poll every `POLL_S = 4.0` s; a symbol unused for `IDLE_STOP_S = 600` s
    stops loading; hour files saved every 60 s; a failed daily file is retried after 30 min.
*   Python caches daily files as parquet under `hub/.cache/binance/aggTrades/` and complete REST hours as
    `.npz` under `hub/.cache/rest_trades/<SYM>/<YYYYMMDDHH>.npz`. No cache exists on the dev machine.

#### 16. `crates/hub-core/src/orderflow/mod.rs`, `calc.rs`, `binance.rs`, `trades.rs`, `crates/hub-core/src/routes/orderflow.rs`, `crates/hub-types/src/orderflow.rs` (C)
*   **Type / Scope**: computation and data loading.
*   **Proposed Changes**: ports of `hub/orderflow_api.py`, `orderflow_trades.py` and `orderflow.py` with
    the facts above. The three routes keep their parameters, defaults, response keys and 502 shape; the
    history function is the hub's `BarsCache` for the `feed` given. No `tpo.rs` (R6). Files are
    grouped by knowledge (architecture, `.paul/phases/02-orderflow/02-ARCHITECTURE.md`): `calc.rs` holds
    every Python number rule and computation, `binance.rs` the futures REST, weight budget and futures
    list, `trades.rs` the trade store, hour cache and daily archive. `feed=mt5|arm`: D-009.
*   **Cache (R5, written default):** a new Rust-only format under `HUB_CACHE_DIR` (default
    `hub/.cache/orderflow/`): `daily/<SYM>-aggTrades-<YYYY-MM-DD>.bin` and `hours/<SYM>/<YYYYMMDDHH>.bin`,
    each a fixed header (magic, version, row count) then little-endian columns (`t_ms` i64, `price` f64,
    `qty` f64, `side` i8, plus `id` i64 in hour files), written to a `.part` file and renamed. The Python
    and Rust caches are not shared; both are re-downloadable. Rejected: real parquet (the `parquet`/`arrow`
    crates add a large dependency tree for a disposable cache). The daily zip is read with the `zip`
    crate (deflate) and parsed as CSV (`transact_time`, `price`, `quantity`, `is_buyer_maker`).

#### 17. `crates/hub-core/tests/orderflow_golden.rs`, `crates/hub-core/tests/fixtures/orderflow/` (C)
*   **Type / Scope**: parity tests, no network.
*   **Proposed Changes**: each fixture holds the captured inputs (futures symbol list, klines pages,
    trades, chart history bars, the frozen clock) and the Python outputs for delta, profile and
    footprint. Symbols: `XAUUSDT` and `BTCUSDT` (real, through a Deriv or Bybit chart ticker each) and one
    estimate-only Bybit symbol whose base is absent from the captured futures list (named in the fixture).
    Values Python rounds (`round(x, 6)` or `round(x, 4)`) must match exactly after the same rounding;
    unrounded floats (`lo`, `step`, `poc`, `vah`, `val`, `row_size`) within 1e-9 relative error.

#### 18. `scripts/capture_orderflow_fixtures.py` (C)
*   **Type / Scope**: one-off tool.
*   **Proposed Changes (R32):** pass 1 downloads the inputs once for fixed past UTC dates (named in the
    fixture) and saves them; pass 2 runs the Python functions with the network replaced by the saved
    inputs (`futures_symbols`, `WEIGHT.get`, the daily-file loader, the history function) and `time.time`
    frozen, then writes the outputs. Re-running pass 2 gives byte-identical fixtures. Deleted in Phase 4
    with the Python server.

### Phase 3: Database and auth

**Phase 3 facts and decisions.** Gate cleared 2026-11-23 (C1 via N5, R3, R4, R13, R18, R21, R27). Where items 19 to 29 below disagree with this block, this block wins.
*   **SQL through sqlx on pgsqlite: the single-table rule (D-014, measured with a sqlx 0.8 probe):**
    typed binds and single-table `SELECT`, `UPDATE`, `DELETE`, `count(*)` and transactions decode
    correctly; `RETURNING`, joins, scalar subqueries and casts return columns mistyped as TEXT (or a
    protocol error), and a unique violation arrives as SQLSTATE `42000` with `UNIQUE constraint
    failed` in the message. So every statement touches one table with no `RETURNING`, join, subquery,
    cast, function or column default; an upsert is `INSERT ... ON CONFLICT DO NOTHING` then a `SELECT`
    by key; uniqueness conflicts are `23505` or that `42000` message. The suite runs on both engines.
*   **Database engine (measured 2026-11-23 with `psql` against pgsqlite 0.0.22, both the owner's
    `~/.local/bin/pgsqlite` and a build of the `fix/insert-select-returning-timestamptz` fork):**
    `uuid`, `timestamptz`, `boolean`, `bytea`, `text`, `bigint` columns; composite `PRIMARY KEY` and
    `UNIQUE` enforced; foreign keys enforced with `ON DELETE CASCADE`; `INSERT ... ON CONFLICT ... DO
    UPDATE ... RETURNING`; `'\x..'::bytea` literals; transactional DDL (`ROLLBACK` undoes `CREATE
    TABLE`). Not available: `decode()`, `pg_advisory_lock()`, and on the owner's binary `DEFAULT now()`
    (rewritten into invalid SQLite). Therefore (D-011): migrations use plain DDL with no function
    defaults, every id and timestamp is written by Rust, and `db.rs` runs its own migration table
    (`schema_migrations(version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL,
    applied_at timestamptz NOT NULL)`, one transaction per file) instead of `sqlx::migrate!`'s runner,
    which needs the advisory lock and a `DEFAULT now()` table. The binary is `PGSQLITE_BIN`, default
    `~/.local/bin/pgsqlite`; gaffa is `~/.local/bin/gaffa`. CI runs pgsqlite and Postgres 17.
*   **Schema (R3), migration `0001_users`:**
    `users(id uuid PRIMARY KEY, email text NOT NULL, email_verified boolean NOT NULL, name text NULL,
    picture text NULL, created_at timestamptz NOT NULL, last_login_at timestamptz NOT NULL)`;
    `identities(provider text NOT NULL, subject text NOT NULL, user_id uuid NOT NULL REFERENCES users(id)
    ON DELETE CASCADE, created_at timestamptz NOT NULL, PRIMARY KEY (provider, subject))` with an index
    on `user_id`; `sessions(id_hash bytea PRIMARY KEY, user_id uuid NOT NULL REFERENCES users(id) ON DELETE
    CASCADE, created_at timestamptz NOT NULL, expires_at timestamptz NOT NULL, last_seen_at timestamptz
    NOT NULL, user_agent text NULL)` with an index on `user_id`; `desktop_auth_codes(code_hash bytea
    PRIMARY KEY, desktop_key_hash bytea NOT NULL, session_id_hash bytea NOT NULL REFERENCES
    sessions(id_hash) ON DELETE CASCADE, expires_at timestamptz NOT NULL)`. Hashes are SHA-256 (32
    bytes); `email` is not unique (one Google account can change its address; the identity is the key).
    Deleting a user removes its identities, sessions and codes by cascade (CUJ 8).
*   **CSRF (C1, decided by N5, D-010):** no synchronizer token. Every non-GET to `/api/me*`, `/auth/*`
    and `/admin/*` must carry `Origin` equal to `PUBLIC_ORIGIN` (403 `{"error": "forbidden origin"}`
    otherwise, a missing `Origin` included) and a JSON body where it has one; the session cookie is
    `__Host-vtv_session`, `SameSite=Lax`. One Tap uses the JavaScript callback, which POSTs
    `{"credential": "<id token>"}` as JSON from the page, so Google's `g_csrf_token` form post is not
    used. The `{user, csrf_token}` return, the stored token hash and the `X-CSRF-Token` header are
    removed from this spec; reload and second-tab writes need nothing extra.
*   **Sessions and keys (R21):** `VTV_SECRET_KEY` is at least 32 random bytes, base64url, required when
    auth is on. Two keys are derived with HKDF-SHA256 (salt `vtv`): info `oauth-cookie-mac` (HMAC-SHA256
    of the sign-in cookie) and info `reserved` (unused in Phase 3). The sign-in cookie
    `__Host-vtv_oauth` holds base64url JSON `{state, nonce, verifier, client, dk_hash, exp}` plus its MAC,
    `HttpOnly`, `Secure`, `SameSite=Lax`, `Path=/`, `Max-Age=600`; it is cleared by the callback. A
    session lasts 30 days from its last slide; it slides (new `expires_at`, `last_seen_at`) at most once
    per 24 h, on the first authenticated request after that. JWKS: cached for its `Cache-Control`
    max-age clamped to 5 min to 24 h; an unknown `kid` refetches at most once per minute.
    `GOOGLE_ISSUER`, `GOOGLE_JWKS_URL`, `GOOGLE_TOKEN_URL` and `GOOGLE_AUTH_URL` are read only in
    builds with the `test-idp` feature; release builds use Google's and accept issuers
    `accounts.google.com` and `https://accounts.google.com` only.
*   **Configuration and state (R18):** `AUTH_ENABLED` default `false` (local mode, D-001: no database,
    no sign-in, `127.0.0.1` only, `/api/logs` served as today). With `AUTH_ENABLED=true`, start-up
    requires `DATABASE_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `VTV_SECRET_KEY` and an
    `https://` `PUBLIC_ORIGIN`, and still binds `127.0.0.1` (ingress through the tunnel); `ADMIN_EMAILS`
    is a comma list, empty by default. Each missing variable fails start-up with its name.
    `AppState` gains `auth: Option<Auth>` (the pool, keys, JWKS cache, config); `app(state)` stays the
    only router. Local-mode logs (D-012): with auth off `/api/logs` and `/api/logs/journal` stay as in
    Phase 1; with auth on they answer 404 and the same handlers serve `/admin/logs` and
    `/admin/logs/journal` to admins only.
*   **Routes (R4):** `GET /auth/me` returns `{"auth": false}` in local mode, `{"auth": true, "user":
    null}` signed out, and `{"auth": true, "user": {"id", "email", "name", "picture"}, "admin": bool}`
    signed in. `POST /auth/google/onetap` `{"credential"}` and `POST /auth/desktop/exchange` `{"code",
    "desktop_key"}` return `{"user": {...}, "admin": bool}` and set the session cookie; a bad token or
    code answers 401 `{"error": "sign-in failed"}` (the reason is logged, not returned).
    `GET /auth/google/start?client=web|desktop&dk=<desktop_key>` redirects (302) to Google;
    `GET /auth/google/callback?state&code` redirects to `/hub.html` (web) or
    `vtradingview://auth?code=<code>` (desktop), or to `/hub.html?signin=failed` on failure.
    `POST /auth/logout` returns 204 and clears the cookie. `DELETE /api/me` returns 204, deletes the user
    (cascade) and clears the cookie. `GET /admin/logs`, `POST /admin/logs/journal` keep the Phase 1
    bodies; `/admin/restart` and `/admin/metrics` are not in Phase 3 (restart has no target since D-001;
    metrics come with the TTFC beacon in Phase 4). Signed-out or non-admin requests to `/admin/*`
    answer 404 `{"error": "not found"}`.
*   **Frontend in Phase 3 (R13, R27):** `hub/auth.js` and the header sign-in control and avatar menu
    ship in Phase 3 (mockup `01-signin-avatar-menu.html`, delete account `04-delete-account.html`). The
    menu has sign out, delete account, and for admins the log panel; sync status (Phase 5) and MT5
    bridges (Phase 7) are absent until their phase. `sync.js` is not loaded before Phase 5. The log
    panel asks `/auth/me` once: `auth: false` polls `/api/logs`, an admin polls `/admin/logs`, anyone
    else gets no log entry and no polling.


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
    the JWKS URL, issuer, auth and token endpoints come from `Config` only with the `test-idp` cargo
    feature (R21 above), which also adds a local signer that `auth.rs` and
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
*   **Proposed Changes**: when `window.vtvDesktop` is present, the sign-in button calls `start_sign_in`
    and shows the waiting overlay; on the deep-link event it POSTs to `/auth/desktop/exchange`
    (amended in Phase 06, D-029/D-030: the flag is `window.vtvDesktop`, not `window.__TAURI__`).

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

## Verification Plan

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
