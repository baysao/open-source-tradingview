# Intermediate Zoom Out Report (Stage 8): vtradingview

*   **Target:** `.paul/design/specs/vtradingview.md` (lines 1 to 915)
*   **Standard:** `zoom-out-standard.md`
*   **Reviewer:** `egm-zoom-out` (fresh context; read the Stage 5 to 7 reports and checked claims against
    `hub/server.py`, `hub/lab.js`, `hub/hub.html`, `hub/vela.bundle.js`)
*   **Date:** 2026-11-23
*   **Context sufficiency:** passes. No fixes were applied between stages.

Elephant checks, confirmed in code: `hub/lab.js:44-46` opens one `WebSocket` per live chart and sends one
bare `{source, symbol, tf}`; `hub/server.py:687-693` cancels the previous stream on each new message (one
subscription per socket); `hub/server.py:346` subscribes Binance `@aggTrade` per symbol (bars built per
timeframe in process); `vela.bundle.js` registers two `beforeunload` listeners including
`this.onUnload`; `hub/hub.html:207` sets the iframe `src`.

## Verdict

The document still solves the root problem at the right altitude; the system shape (one router, one
port, FeedHub, sessions, tunnel) needs no change. Three structural decisions inside that shape were
missed or only partly seen earlier: the websocket protocol and its limits (N1), the FeedHub sharing key
(N2), and when the sign-in merge runs relative to the chart iframe (N3). The contract suite cannot prove
G1 as designed (N4). The cheapest fix for Blocker C1 is to drop the token (N5).

## Earlier findings still open

All Stage 5 findings (F1 to F12), Stage 6 F2 to F5 and Nits, Stage 7 Blockers C1, C2 and Majors C3 to
C25, and escalation E1 (self-hosting) remain open. New evidence strengthens Stage 5 F1: the Python
websocket side is already asyncio (`hub/server.py:687` `async def handler`, `asyncio.run` at line 1070);
only HTTP uses `ThreadingHTTPServer` (lines 22, 1039), so the unweighed async-Python alternative is
smaller than Stage 5 assumed.

## New findings

| Id | Severity | Finding | Simplest fix |
|---|---|---|---|
| N1 | Major | **The websocket protocol is silently redesigned.** Today: one socket per chart, one bare subscribe message, replies `history`, `status`, `bar`, `error`. The spec assumes a multiplexed protocol (16 subscriptions per connection, subscribe and unsubscribe types) while G1 promises unchanged shapes and item 12 calls the frontend change one line. With the real client, 4 sockets per IP breaks the spec's own 8-chart layout. | Keep today's protocol (one subscription per socket); drop the per-connection cap; set the per-IP socket limit from charts times tabs. |
| N2 | Major | **FeedHub shares on the wrong key.** Upstreams are per-symbol trade or tick streams (Binance `@aggTrade`, Deriv `ticks`, Bybit trades bucketed by `TF_SEC`); keying on `(source, symbol, tf)` opens up to one upstream per timeframe and makes the cap count timeframes. | One upstream per `(source, symbol)`, per-timeframe bar builders fanning out beneath it; the cap counts symbols (makes C14's exhaustion about 10 times harder). |
| N3 | Major | **The sign-in merge races Vela's save-on-unload.** Writing server values then reloading the iframe fires Vela's `beforeunload` save, which overwrites the server value; the `setItem` wrapper uploads the stale value back. The merge's own writes also echo through the wrapper. | Finish the merge before the parent sets `frame.src` (or unload the iframe first, then write); drop "reload once after the merge"; exclude merge writes from upload. G2 must account for the short delay for signed-in users. |
| N4 | Major | **The contract suite is circular.** `hub-types` is written from Python's output and serde ignores unknown fields, while `Option` absorbs missing ones, so a Rust server that drops or adds fields still passes; G1 is the gate that deletes Python. | Record raw JSON from both servers for the same requests and diff key sets and types; for the websocket compare `type` values and field sets. `hub-types` is the model, not the oracle. |
| N5 | Major | **Remove the CSRF token rather than issue more.** `__Host-`, `SameSite=Lax`, non-GET for every change and a required `Origin` equal to `PUBLIC_ORIGIN` already defend against CSRF in modern browsers; the synchronizer token creates C1. Google's `g_csrf_token` on One Tap stays. | Drop token issue, storage, header and reload cases. |
| N6 | Nit | The Elephant's K4 fix, read as "Rust POSTs the exchange", would put the cookie in Rust's HTTP client, not the webview store. | `start_sign_in` returns `desktop_key` to the page; the two-permission claim still holds. |
| N7 | Nit | **Two tabs defeat the 409 check.** `vtv:ver:<key>` is in shared `localStorage`, so tab B's stale save carries A's new version and overwrites without a 409. | Keep the last-seen version per tab (memory or `sessionStorage`). |
| N8 | Nit | Keep C11 and C16 mitigations small: `auto_select: false` to match "one click"; if TTFC persists, rows in the existing database, no metrics service. | |
| N9 | Nit | `sync.js` may not be needed inside the iframe: same-origin writes fire `storage` events in the parent. Only once N7 is in place, since other tabs fire the same events. | |

## Do the Stage 6 and 7 mitigations change the architecture?

No change to the system shape: C2 to C13 and C17 to C25 are document or UI specification; C4 and C5 add
at most one product stylesheet; C1 shrinks if fixed as N5. Structural changes inside the shape: C14 and
C15 need N1 and N2; Stage 6 F4 needs a desktop-key decision (N6); CUJ 3 needs N3.

## Elephant notes (orchestrator, not the reviewer)

*   **N6 reads a fix that was not proposed.** The goldfish report's K4 edit does not have Rust POST the
    exchange: Rust navigates the webview itself to `/auth/desktop/complete?code=...&dk=...`, a top-level
    navigation, so the server's `Set-Cookie` lands in the webview's own cookie store. The reviewer's
    objection does not apply to that mechanism; its alternative (return the key to the page) is also
    workable. Both stay open for the owner.
*   **N8 versus C11.** `auto_select: false` contradicts the owner's stated requirement "auto login"
    (PLANNING.md). The goldfish report keeps `auto_select: true` and fixes CUJ 2's wording; that stands.
*   **N5 versus C1.** Both fixes remove C1. N5 is smaller; it relies on `Origin` being present on every
    non-GET, which current browsers send for `fetch` POST, PUT and DELETE. A request with no `Origin`
    header must then be refused, which should be stated.
