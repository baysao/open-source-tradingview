# Implementation Readiness Report (Stage 9, round 1): vtradingview

*   **Target:** `.paul/design/specs/vtradingview.md`, DetailedRange lines 563 to 914
    (`## Detailed Implementation Plan`); FeatureRange 1 to 562 read as context only
*   **Reviewers (direct dispatch, parallel, fresh context, no GoldfishContext):**
    *   `egm-implementation-readiness`: `implementation-readiness-standard`, completed (3 Blocker, 16 Major, 1 Nit group)
    *   `egm-codebase-details`: `implementation-readiness-standard`, completed (5 Blocker, 19 Major, 9 Nit)
    *   `egm-style`: persona vectors, completed (0 Blocker, 7 Major, 22 Nit)
*   **Date:** 2026-11-23

Elephant checks, confirmed in code: one socket per chart and untyped subscribe (`hub/lab.js:43-46`,
`hub/server.py:687-704`); no server-side TPO (no `tpo` in `hub/*.py`; TPO is browser-side in
`hub/orderflow_tpo_type.js`); order-flow cache is parquet under `hub/.cache/binance/`
(`hub/orderflow.py:18, 32`); log ring is `deque(maxlen=12000)` (`hub/logbus.py:17`); favourite-bar keys
`tradexDrawFavs`, `tradexFavBar` (`hub/drawing_favbar.js:7`) are outside the sync allow-list.

Verdict: **not ready.** The plan lists files and phases well, but it fails the Zero-Decision Rule in
three places that decide everything downstream: the websocket protocol, the database schema, and the
shapes of the new HTTP routes. It also misstates current behaviour in four places.

## Blockers

| Id | Finding | Items | Raised by |
|---|---|---|---|
| R1 | **Websocket protocol contradicts the code and G1.** Today: one untyped `{source, symbol, tf}` per socket, each new message replaces the stream; server sends `history`, `bar` (`sent`, `trade_ms`, `lag_ms`), `status`, `error` with `msg`. Item 3 lists subscribe, unsubscribe, bar, tick, error with `kind`, "copied from `hub/server.py`", which is false. Item 12 changes only `wsUrl()`; item 11's `/ws` case cannot pass on both servers. Same as Stage 8 N1. | 3, 6, 7, 11, 12 | readiness, codebase |
| R2 | **Per-IP limit of 4 sockets breaks the 8-chart layout** (`hub/hub.html:140`, `hub/vela.html:55-63`); the 16-per-connection cap never applies because the client does not multiplex. | 6, 12 | readiness, codebase |
| R3 | **Schemas have no columns.** Item 19 names four tables with no columns, types, nullability, indexes or foreign keys; item 49 lists names without types; the UUID type on pgsqlite is unchosen; `desktop_auth_codes` has no column binding the code to `desktop_key`; cascades for CUJ 8 not stated. | 19, 49 | readiness, codebase |
| R4 | **New routes have no request or response shapes:** `/auth/google/onetap`, `/auth/desktop/exchange`, `/auth/me` (incl. the "auth disabled" field item 24 needs), `/admin/logs`, `/admin/restart`, `/admin/metrics`, `/api/me/state` GET and PUT (body, version location, 409 body, limit status codes), `DELETE /api/me`; `/api/me/mt5*` is a wildcard, not an API; no replacement for `POST /api/logs/journal`. | 22, 32, 38, 50 | readiness |
| R5 | **Order-flow cache layout misstated.** Python caches parquet under `hub/.cache/binance/aggTrades/` and `hub/.cache/rest_trades/<SYM>/`; `DATA_DIR` does not exist; no parquet crate in item 1. "Usable by the other" cannot hold as written. | 16 | readiness, codebase |
| R6 | **`tpo.rs` ports Python code that does not exist.** TPO is computed in the browser; no `/api/orderflow/tpo` route; item 17 cannot produce TPO fixtures. | 16, 17 | codebase |
| R7 | **Log ring size wrong:** plan says 2,000 lines; actual ring is 12,000 (2,000 is the per-message truncation and throttle cap). | 8 | codebase |

## Majors

### Current behaviour the plan omits or misstates
*   **R8 (codebase M1):** logbus and feedlog parity undefined: rotating `hub/logs/hub.log` 2 MB x 3,
    history reload at boot, `HUB_TAIL_FILES` tailing with offsets, silence checks and `PROBLEMS`, stdout
    `Tee`, the `snapshot` response shape, feedlog stream watchers.
*   **R9 (codebase M2):** `/api/health` clock offset (SNTP to `time.windows.com`) and per-symbol lag
    stats are not planned; item 10's "same start-up lines" cannot be true on one port; exact new lines not
    given (`AGENTS.md:27-31` quotes them).
*   **R10 (codebase M3):** order-flow constants omitted: Binance weight budget 1900/2400 per minute with a
    400 reserve and stop on 429/418, `MAX_PAST_DAYS=2`, `MAX_REST_HOURS=30`, `POLL_S=4.0`,
    `IDLE_STOP_S=600`, futures symbol list refresh, `real_symbol` mapping, `INTRABAR`, `MAX_ROWS=20000`;
    the source is Binance USD-M futures.
*   **R11 (codebase M5):** static serving: `/` serves `hub/index.html` (a redirect page), not `hub.html`;
    static root, percent-decoding, `Cache-Control` headers and the JSON 404 body unspecified.
*   **R12 (codebase M6, M8):** `POST /api/logs/journal` and `GET`/`POST /api/restart` (Origin, JSON
    content type, 300,000-byte cap) not ported in Phase 1; `/admin/restart` semantics undefined; the
    Restart button also reconnects every other feed (`hub/hub.html:191-201`), so "remove lines 195-196"
    is ambiguous.
*   **R13 (codebase M7):** `log_panel.js` polls `/api/logs?brief=1` for every visitor; after the move to
    `/admin/logs` every non-admin page polls a 404 forever unless gated on `/auth/me`.
*   **R14 (readiness 4, codebase M9, M10):** what Rust returns for `mt5`, `arm` and `ws_port` in Phase 1
    is undecided; `source=mt5` is reused in Phase 7 while today it means Local MT5; frontend and
    `AGENTS.md` references not removed. Ties to escalation E1 (self-hosting).
*   **R15 (codebase M15):** `--onAccent` has no consumers: every on-accent text reads `--accentInk`
    (`hub/hub.html:17, 72-73`, `log_panel.js:17`, `drawing_favbar.js:25`).
*   **R16 (codebase M18):** `tradexDrawFavs` and `tradexFavBar` are not in the sync allow-list.

### Decisions left to the implementer
*   **R17 (readiness 5, codebase M11):** dependency choices missing or unversioned: sqlx (features),
    dotenvy, zip, csv, parquet or arrow, JWT/JWKS, sha2, rand, uuid, chacha20poly1305, LRU, rate limiter,
    DNS resolver for the vetted-address resolver, tauri 2 and its plugins; versions for tokio-tungstenite,
    reqwest, serde, tracing.
*   **R18 (readiness 7):** `app(Config)` vs `app(cfg, state)`; `AppState` fields; `Config` defaults;
    which variables are required from which phase; no item implements `AUTH_ENABLED`.
*   **R19 (readiness 8, codebase M17, style 5):** no CSP string (inline scripts in four pages, Google Fonts
    injection, Vela Workers and `createObjectURL`, GIS); no rate, burst or route scope for the limiter; no
    request-id layer; phase conflict (PLANNING Phase 4 vs item 9 Phase 1).
*   **R20 (readiness 9, style 13):** LRU capacity, newest-window TTL, backoff schedule, ping and pong
    timeout; cache key uses `end` but the client sends `to`, and `limit` is missing from the key.
*   **R21 (readiness 10):** session slide frequency; state/nonce/PKCE cookie name, TTL, MAC; key encoding,
    length and separation between MAC and encryption; JWKS TTL; `GOOGLE_ISSUER` configurable yet
    "refused" in release.
*   **R22 (readiness 11):** the admin websocket log channel has no item; new redaction patterns not
    listed; TTFC histogram buckets and beacon limit missing.
*   **R23 (readiness 13):** sync edge cases: no server delete for `removeItem`; backups past the 256-key
    limit; two same-day backups collide on `backup:<key>:<date>`; signed-out wrapper behaviour; retry
    schedule; allow-list patterns not written; `chartHubDraft` rule not in item 40.
*   **R24 (readiness 14):** how a user's bridges reach the anonymous, synchronous `/api/symbols` picker;
    `/wait` polling interval and failure; bridge-to-`Bar` mapping; MT5 subscribe format.
*   **R25 (readiness 18, codebase M16):** desktop origin compile-time or runtime; how the page gets
    `desktop_key`; Tauri 2 permission names.
*   **R26 (readiness 19):** backup off-host destination, tool and scheduler (blocked on Q1).

### Phasing, CI and tests
*   **R27 (readiness 12):** item 25 loads `sync.js` in Phase 3 but it is created in Phase 5 (404 in
    Phases 3 and 4); PLANNING puts header sign-in UI in Phase 5, items 24-25 in Phase 3; no later phase
    marks `Cargo.toml` (M); adding `crates/desktop` needs webkit2gtk on CI runners (codebase M12).
*   **R28 (readiness 15, codebase M19, style 6):** `dev.sh` and `Procfile` defined by reference to another
    repo; pgsqlite source (`~/project` or `vendor/`) and population undecided; no `DATABASE_URL` for
    port 55432.
*   **R29 (readiness 16, codebase M13, style 1):** CI setup missing: Postgres 17 service, pgsqlite build,
    `test-idp` build, Playwright (no package or config), `cargo llvm-cov`, `cargo-audit` install,
    `requirements.txt` for the Python contract job; `cargo clippy -D warnings` is invalid (needs
    `-- -D warnings`); behaviour of `contract.rs` when `CONTRACT_BASE_URL` is unset.
*   **R30 (codebase M14):** contract cases depend on live upstreams that block CI data-centre IPs (Bybit,
    Binance) or close at weekends (Deriv); no fixture or skip rule.
*   **R31 (readiness 17, Stage 8 N4):** no named tests for static traversal (`%2e%2e`, `%5c`), FeedHub
    cap and grace, rate limiter, redaction, websocket `Origin`, `--onAccent`, merge table rows; the
    contract check needs `deny_unknown_fields` and required fields, or it proves nothing.
*   **R32 (codebase M4):** fixture capture is not deterministic: Python calls the network for the futures
    list, klines and trades; dates and the estimate-only symbol not named; PAXGUSDT named while
    `real_symbol` maps gold to `XAUUSDT`.

### Precision (style)
*   **R33 (style 2-4, 7):** ambiguous bare file names (`auth.rs` twice, `admin.rs` three times); load test
    "realistic subscriptions" and undefined N; state limits without values; home-directory paths break
    the "paths from repository root" rule.

## Nits (grouped)

*   Codebase N1 to N9: `.paul/` is untracked today (needs `git add`); Python also loads `hub/.env`;
    missing names in `.env.example` (`DATA_DIR`, `GOOGLE_JWKS_URL`, `GOOGLE_ISSUER`, `GOOGLE_TOKEN_URL`,
    `TUNNEL_TOKEN`, `CONTRACT_BASE_URL`, `BRIDGE_TOKEN`, `BRIDGE_BIND`, `FEED_MAX_SYMBOLS` in `Config`);
    Python uses fixed sleeps, not backoff; `requirements.txt` would be empty (mark D) and item 35 vs
    PLANNING retirement timing disagree; Tauri `custom-protocol` and `tray-icon` are Cargo features;
    `sqlx::migrate!("../../migrations")` path; allow-list omits `notices.html`, `ttfc.js`,
    `settings_mt5.js`; item 26 data sources unnamed.
*   Readiness 20: `desktop/` vs `crates/desktop/` config path; `requirements.txt` contents;
    `.gitignore` cache and log entries after retirement; no Windows launcher after `start.bat` is deleted.
*   Style 8 to 29: unversioned pins and "toolchain pin" without a version; time-relative "today" and
    source line numbers; `{ok, ...}` ellipsis; "later" and unnamed "auth keys"; filler words; mixed
    British and American spelling; `55432` vs `PGSQLITE_PORT`; `PUBLIC_ORIGIN` vs hard-coded host;
    undefined "smoke test", "CUJ", "vetted-address resolver", "beacon", "green"; privacy page content;
    "Owner-only access." fragment; mixed states and actions list; "each new screen" unnamed; coverage
    method; label capitalisation; provenance footer. Style also flags the load test vs the 4-per-IP cap.

## Carried, still open

Stage 7 Blockers C1 (CSRF on reload) and C2 (no Risks section); Stage 8 N1 to N5 (R1 and R2 here
repeat N1; R31 repeats N4); escalation E1 (self-hosting); the fixes proposed in
`goldfish_review_report.md`, none applied.

## Round status

Not clean: 7 Blockers and 26 Majors in the DetailedRange, on top of the open FeatureRange findings.
