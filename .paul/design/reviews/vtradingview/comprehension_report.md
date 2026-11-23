# Comprehension Report (Stage 6): vtradingview

*   **Target:** `.paul/design/specs/vtradingview.md`, lines 1 to 915 (FeatureRange = whole document;
    DetailedRange N/A, preset "feature spec only")
*   **Reviewer:** `egm-comprehension` (fresh context, no GoldfishContext, linked files not opened)
*   **Date:** 2026-11-23

## What this proposes

TradeX is a charting app that today runs only on a user's own machine. The proposal makes it a public
website at vtradingview.com and adds a thin Tauri 2 desktop app that opens that site in a native window.
A Rust/Axum server replaces the Python server. It returns the same `/api/*` response shapes, and live data
moves onto one port at `/ws`. A shared FeedHub opens one upstream subscription per (source, symbol,
timeframe) for all viewers. The order-flow maths is ported from numpy/pandas to plain Rust and proven
equal with golden fixtures.

Google sign-in (One Tap and a button flow on the web, the system browser plus a `vtradingview://` deep
link on desktop) creates server-side cookie sessions. A `sync.js` layer copies allow-listed `localStorage`
keys to a per-user `user_state` table. It uses version numbers, returns a 409 on conflict, and at sign-in
the server's copy wins while the local copy is kept as a backup. Signed-in users can register their own
HTTPS MT5 bridge, with an SSRF guard in front. An admin panel shows feed health and the time-to-first-chart
figure.

The database speaks the Postgres protocol: pgsqlite over a SQLite file by default, real Postgres by
changing `DATABASE_URL`. The host is a Linux machine run under "gaffa", reachable only through a
Cloudflare Tunnel. The work is split into seven phases.

## How the existing system works, per this document

A Python process, `hub/server.py`, binds 127.0.0.1. It serves static pages from an allow-list on port
8770 and live market data on a websocket at port 8765. Feeds come from Deriv, Bybit and Binance, with one
upstream connection per need rather than one shared across viewers. The standard-library threaded HTTP
server runs pandas inside request handlers for order flow (delta, footprint, profile, TPO). That code uses
Binance trade archives cached under `DATA_DIR/orderflow/`, and falls back to a labelled estimate from
candles. A log bus (`logbus.py`, `feedlog.py`) applies redaction and serves `/api/logs`, and `/api/restart`
restarts a remote MT5 link.

The frontend is vanilla JS. `hub.html` is a parent page that hosts one chart-engine page in an iframe
(Vela, Lightweight Charts or KLineChart). All user work lives in one browser's `localStorage`
(`chartHub*` keys plus Vela keys whose names are built at run time).

MT5 data comes from `bridge/mt5_bridge.py`, which the hub polls on its `/wait` route. The document also
mentions "remote-MT5 SSH settings" and a remote-MT5 restart button but never explains them, so the
reviewer could not fully paraphrase how MT5 works today (Finding 3).

## Findings

| # | Severity | Scope | Finding |
|---|---|---|---|
| 1 | Major | Q8, CUJ 3, "Sign-in merge (decided ...)", Alternative G | The merge rule is called open (Q8, CUJ 3) and decided (Technical Plan, Alternative G). A reader cannot tell which. Same defect as Stage 5 Finding 7. |
| 2 | Major | "gaffa" (lines 30, 196, 372, 535, 549, 783); "pricehub" (372, 549, 646) | The process supervisor that delivery and rollback rest on is never defined; "pricehub" is assumed known. A reader cannot tell how processes are started, restarted or rolled back. |
| 3 | Major | Lines 358-359, 486-490, 724, 741 | Today's MT5 setup (where the bridge runs, what "remote MT5" and its SSH settings do, what the restart button restarts) is assumed, not described, so the loss from removing it cannot be judged. Overlaps Stage 5 Finding 2. |
| 4 | Major | Desktop sign-in (283-291, 386-391, 830-833, 843-844) | The `desktop_key` is created in Rust and "kept in memory", yet the window's JavaScript must POST `{code, desktop_key}`. The document never says how the page gets the key (command return value, deep-link event payload, or Rust does the POST). This matters for the two-permission security claim. |
| 5 | Major | Lines 398, 425 | "D4" and "the lint's readability floor" are design-track terms never defined in the document. |
| 6 | Nit | Header line 7 | "Seed" and "draw" are jargon, only explained in Alternative I. |
| 7 | Nit | Lines 218 and 591 | Two signatures for the entry point: `app(Config)` and `app(cfg: Config, state: AppState)`. |
| 8 | Nit | Line 223 | `desktop` depends on "shared constants": which crate holds them is not said. |
| 9 | Nit | Line 220 | "The reference template" is used before Alternatives A and B introduce it. |
| 10 | Nit | Line 234 | "Requests without the header are treated as local": what "local" grants (rate limits, IP key, trust) is undefined, and it is security-relevant. |
| 11 | Nit | Q5 (line 103) and item 42 (line 815) | The Vela key inventory is needed "before Phase 5 is planned" but is itself a Phase 5 deliverable. |
| 12 | Nit | Lines 368 and 905 | The in-memory TTFC histogram resets on every restart, including releases; a full week of samples for G2 is not guaranteed. |
| 13 | Nit | CUJ 2 (line 130) and line 272 | "Session" and "tab" give different scopes for the One Tap dismissal. |
| 14 | Nit | Naming | TradeX, "the hub" and vtradingview are not defined against each other; "vela/lw/kline" resolved only at line 768; `hub/index.html` called an engine page without saying which. |
| 15 | Nit | Line 252 | "Short TTL for the newest page": "page" means newest bar window; no TTL value. |
| 16 | Nit | Lines 332-336 | Whether `backup:*` keys live only on the server or also in `localStorage`, and whether the sync allow-list covers them, is not stated. |

## Reviewer's overall view

The document is largely self-contained and its parts connect; every section except today's MT5 setup
could be paraphrased. Majors: the contradictory merge rule, the undefined runtime (gaffa, pricehub), the
undescribed current MT5 setup, the missing step for how the desktop page gets `desktop_key`, and the
undefined "D4".

## Elephant notes (orchestrator, not the reviewer)

*   Finding 4 is a real design gap, not wording: either the deep-link event carries the key to the page
    (the remote page then holds it), or Rust performs the exchange POST and only sets the cookie. The
    second keeps the key out of the remote page and fits the "two permissions" claim. To resolve at
    Stage 7.
*   Findings 1 and 3 repeat Stage 5 Findings 7 and 2: two independent readers hit the same holes.
