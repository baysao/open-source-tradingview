# Decisions

<!-- One entry per decision, newest last. Never edit a decided entry: supersede it
with a new entry that names the old id. -->
## D-001: Local mode for charts only; MT5 only through bridges
- **Date:** 2026-11-23  **Plan:** 01-01  **Status:** decided
- **Decision:** `hub-server` keeps a supported local mode: `AUTH_ENABLED=false`, no database, bound to `127.0.0.1` only (refuses any other bind with auth off). Local mode serves charts and the free feeds. Local MT5 (`LOCAL_MT5`) and SSH Remote MT5 (`arm`) are not ported; MT5 returns in Phase 07 through per-user bridges, where the SSRF guard allows loopback bridge URLs only when auth is off. `/api/symbols` drops `mt5` and `arm`; `ws_port` stays one phase with value 8770 until `hub/lab.js` derives the URL. `/api/restart` and the arm SSH code are not ported; the Restart button keeps reconnecting the free feeds. `start.bat` and `start.sh` are rewritten to run the Rust binary. The last Python release is tagged `python-final`.
- **Why:** Phases 01 and 02 must run without a database and Google keys anyway, so local mode costs almost nothing; it keeps the AGPL self-hosted promise; porting Local MT5 and arm would be replaced by Phase 07 bridges. Rejected: (a) drop self-hosting (breaks existing users, no gain), (b) port both MT5 paths in Phase 01 (throwaway work).
- **Evidence:** owner chose "Accept (c)" at the 01-01 gate, 2026-11-23; resolves escalation E1 in `.paul/design/reviews/vtradingview/goldfish_review_report.md`.

## D-002: Keep today's websocket protocol; share upstreams per symbol
- **Date:** 2026-11-23  **Plan:** 01-01  **Status:** decided
- **Decision:** `/ws` keeps the Python protocol exactly: one subscription per socket; the client sends one untyped `{source, symbol, tf}` and a new message replaces the stream; the server sends `history` (`bars`), `bar` (`bar`, `sent`, optional `trade_ms`, `lag_ms`), `status` (`msg`) and `error` (`msg`), plus one additive optional field `kind: "capacity"` on `error` for the global-cap refusal. Limits: 24 sockets per client IP; no per-connection subscription cap. FeedHub opens one upstream per `(source, symbol)` with a bar builder per timeframe beneath it; `FEED_MAX_SYMBOLS` counts `(source, symbol)` pairs. `hub/lab.js` changes only `wsUrl()`.
- **Why:** the contract suite can compare `/ws` against Python only if the protocol is unchanged, which is the gate for deleting Python; upstreams are per-symbol trade or tick streams (`hub/server.py:346` Binance `@aggTrade`, Deriv `ticks`), so keying on timeframe opened duplicate upstreams. Rejected: one multiplexed socket per tab (rewrites `lab.js`, breaks parity for `/ws`).
- **Evidence:** owner answered "your assumption" to the recommendation at the 01-01 gate, 2026-11-23; resolves R1, R2 (implementation_readiness_report.md), N1, N2 (intermediate_zoom_out_report.md).

## D-003: Keep `mt5` and `arm` as "off" in /api/symbols (amends D-001)
- **Date:** 2026-11-23  **Plan:** 01-01  **Status:** decided (owner approved with the Phase 01 plans, 2026-11-23)
- **Decision:** `/api/symbols` keeps the keys `mt5` and `arm` with the constant value `"off"` (exactly what Python sends with both disabled) and keeps `ws_port` (8770). `hub/lab.js` keeps reading them; only `wsUrl()` changes. This replaces the D-001 sentence "`/api/symbols` drops `mt5` and `arm`" and the spec item 12 removal of `LabMT5Status` / `LabARMStatus`.
- **Why:** `hub/lab.js:95-97` warns in the console unless each value is `"ok"` or `"off"`, and the browser edit is limited to `wsUrl()` (D-002). All three /architect candidates and the cross-judge reached this independently.
- **Evidence:** `hub/lab.js:95-97`; `hub/server.py:129`, `747`; `.paul/phases/01-rust-parity/01-ARCHITECTURE.md` "Synthesis decision".

## D-004: Publish the closed bar once on roll-over
- **Date:** 2026-11-23  **Plan:** 01-02  **Status:** decided (owner approved with the Phase 01 plans, 2026-11-23)
- **Decision:** when a tick opens a new bucket, the closed bar's final state is sent as one `bar` frame before the new bar, bypassing the 50 ms throttle. No new message type or field.
- **Why:** Python's per-viewer throttle has no trailing flush, so a bar's last updates before close can be lost (`hub/server.py:365`); this fixes closing OHLC on every chart. It adds at most one frame per bar close, which the contract suite (types and field sets, not counts) does not see. Rejected: a trailing-flush timer (a timer on the hot path).
- **Evidence:** arena graft from candidate A, cross-judge confirmation; `.paul/phases/01-rust-parity/01-ARCHITECTURE.md`.

## D-005: Pin crate versions compatible with axum 0.8, not the absolute latest
- **Date:** 2026-11-23  **Plan:** 01-01  **Status:** decided (APPLY deviation, recorded at UNIFY)
- **Decision:** the workspace pins tower-http 0.6, reqwest 0.12 (rustls), tokio-tungstenite 0.26, lru 0.12 and toml 0.8 alongside axum 0.8 and tokio 1; `Cargo.lock` is the exact pin. A newer major of any of these is taken only in a plan that builds, lints and passes the contract suite with it.
- **Why:** the plan text said "latest release on the day", but the latest majors (tower-http 0.7, reqwest 0.13, tokio-tungstenite 0.30, lru 0.19, toml 1.x) were not confirmed compatible with axum 0.8; a mixed tree compiles two copies of shared crates or fails. Rejected: chasing the newest majors in the first plan.
- **Evidence:** `.paul/phases/01-rust-parity/01-01-SUMMARY.md` deviation 1; clean `cargo build`, `clippy -D warnings`, `cargo test` and 18/18 contract cases on this set.

## D-006: Logging convention: tracing target `server` or `feed`, provider in a `src` field
- **Date:** 2026-11-23  **Plan:** 01-02  **Status:** decided (APPLY deviation, recorded at UNIFY)
- **Decision:** server events log with `tracing` target `server`; feed events with target `feed` and a field `src` set to the provider (`deriv`, `bybit`, `binance`); an optional field `every` gives the throttle in seconds. 01-03's `RingLayer` maps target `feed` plus `src` to the log-panel source `feed.<src>`, and target `server` to `server`.
- **Why:** `tracing` targets must be static strings, so a target per provider (`feed.binance`) cannot be built from a runtime value; one field keeps a single writer convention that 01-01 and 01-02 code already follow. Rejected: three hard-coded targets per call site (duplicated match arms).
- **Evidence:** `crates/hub-core/src/feeds/{hub,upstream}.rs`; `.paul/phases/01-rust-parity/01-02-SUMMARY.md` deviation 4.

## D-007: Per-IP rate limit is a small in-crate token bucket, not tower_governor
- **Date:** 2026-11-23  **Plan:** 01-03  **Status:** decided (APPLY deviation, recorded at UNIFY)
- **Decision:** `net::RateLimiter` is a per-client-IP token bucket (20/s, burst 40, map pruned past 10,000 keys) applied to `/api/*` by `net::rate_limit`, keyed by `client_ip()` (CF-Connecting-IP only from a loopback peer), answering 429 `{"error":"too many requests"}` with `Retry-After`.
- **Why:** about 40 lines with a pure, unit-tested `take(ip, now)`, keyed by our own `ClientIp` rule instead of a governor key extractor, and one dependency fewer. Rejected: `tower_governor` (extra crate and a second IP-extraction rule to keep in sync).
- **Evidence:** `crates/hub-core/src/net.rs` `token_bucket` test; burst of 45 gave 40 x 200, 5 x 429, `Retry-After: 1` (01-03-SUMMARY AC-4).

## D-008: Order-flow cache is a new Rust-only column format, not parquet
- **Date:** 2026-11-23  **Plan:** 02 gate (R5)  **Status:** decided (written default at plan time; owner may override)
- **Decision:** Rust caches Binance daily trade files and complete REST hours under `HUB_CACHE_DIR` (default `hub/.cache/orderflow/`) as a fixed header plus little-endian columns (`t_ms`, `price`, `qty`, `side`, and `id` for hours), written to `.part` then renamed. The daily zip is read with the `zip` crate and parsed as CSV. Python and Rust caches are not shared.
- **Why:** both caches are disposable and re-downloadable, none exists on the dev machine, and real parquet means the `parquet`/`arrow` dependency tree for no user-visible gain. Rejected: (a) real parquet shared with Python, (b) keeping Python's paths with a different format (two writers, one path).
- **Evidence:** `.paul/design/specs/vtradingview.md` Phase 2 facts and item 16; `hub/orderflow.py:18-48`, `hub/orderflow_trades.py:22,113-151`.

## D-009: Order flow with `feed=mt5|arm`, and Python quirks ported verbatim
- **Date:** 2026-11-23  **Plan:** 02 architecture  **Status:** decided (written default at plan time; owner may override)
- **Decision:** `feed=mt5|arm` parses to `Feed::Off`; a request that never calls the chart history (a real-source delta) answers 200 as in Python; anything that calls history answers 502 `RuntimeError: <feed> history is not available in this server (D-001)`. Python quirks are ported as they are before the parity gate (a hole inside the cached lower-bar span is never refetched; the footprint key `bt*1e8 + row`). Two deliberate differences that cannot change a 200 body: the rows cache key includes the feed and is LRU-bounded, and `va=nan` answers 502 `ValueError` (Python writes invalid JSON `NaN`).
- **Why:** MT5 and arm do not exist in the Rust server (D-001); Python's own text for them depends on the machine. Bit-exact parity is only provable if the quirks are kept until the golden suite is green.
- **Evidence:** `.paul/phases/02-orderflow/02-ARCHITECTURE.md` decisions 6 and 7; `hub/server.py:925-926`, `hub/orderflow_api.py:120-140`.

## D-010: CSRF by Origin and SameSite, no synchronizer token (C1 resolved by N5)
- **Date:** 2026-11-23  **Plan:** 03 gate  **Status:** decided (written default at plan time; owner may override)
- **Decision:** every non-GET to `/api/me*`, `/auth/*` and `/admin/*` requires `Origin` equal to `PUBLIC_ORIGIN` (a missing `Origin` is refused) and JSON where there is a body; the session cookie is `__Host-vtv_session`, `SameSite=Lax`; One Tap uses the JavaScript callback (JSON POST), not Google's form post. No CSRF token is issued, stored or sent.
- **Why:** the token returned only at sign-in broke every write after a reload or in a second tab (C1); N5 showed `Origin` plus `SameSite=Lax` on `__Host-` cookies already defends modern browsers, and removes the token's state. Rejected: a token re-issued by `/auth/me` (more state, same protection), a double-submit cookie (needs JS-readable cookie).
- **Evidence:** `.paul/design/reviews/vtradingview/intermediate_zoom_out_report.md` N5; `critic_report.md` C1; spec "Phase 3 facts and decisions".

## D-011: Migrations run on our own runner with plain DDL (pgsqlite limits)
- **Date:** 2026-11-23  **Plan:** 03 gate (R3)  **Status:** decided (measured)
- **Decision:** `db.rs` applies embedded `migrations/*.up.sql` in name order, one transaction each, recorded in `schema_migrations` (plain DDL, no function default); migrations use no database functions or defaults; ids (UUID v7) and timestamps are written by Rust. The pgsqlite binary is `PGSQLITE_BIN`, default `~/.local/bin/pgsqlite` (the owner's install); gaffa is `~/.local/bin/gaffa`.
- **Why:** measured on pgsqlite 0.0.22: no `pg_advisory_lock` (needed by sqlx's migrator), no `decode()`, and on the owner's binary `DEFAULT now()` becomes invalid SQLite, which sqlx's `_sqlx_migrations` table uses. Constraints, cascades, upsert with RETURNING and transactional DDL all work. Rejected: `sqlx::migrate!` with `set_locking(false)` (still the `DEFAULT now()` table), requiring the fork build (owner pointed at the installed binary).
- **Evidence:** probe scripts in the session scratchpad; spec "Phase 3 facts and decisions".

## D-012: Logs stay public in local mode and move behind admin with auth on
- **Date:** 2026-11-23  **Plan:** 03 gate (R13, R18)  **Status:** decided (written default)
- **Decision:** with `AUTH_ENABLED=false` (local, `127.0.0.1`), `/api/logs` and `/api/logs/journal` stay as in Phase 1; with auth on they answer 404 and `/admin/logs`, `/admin/logs/journal` serve admins only (404 to everyone else). The log panel asks `/auth/me` once and polls only the route it may use. `/admin/restart` and `/admin/metrics` are not built in Phase 3.
- **Why:** local mode has no accounts, and its only user is the machine owner (D-001); on the hosted site logs expose server internals (PROJECT key decision). Polling a 404 forever from every visitor (R13) is removed.
- **Evidence:** spec "Phase 3 facts and decisions"; `hub/log_panel.js` polls `/api/logs?brief=1` today.

## D-013: Phase 3 ships the sign-in UI; sync waits for Phase 5
- **Date:** 2026-11-23  **Plan:** 03 gate (R27)  **Status:** decided (written default)
- **Decision:** `hub/auth.js` and the header sign-in control and avatar menu (sign out, delete account, admin log panel) ship in Phase 3 from mockups 01 and 04; `sync.js` is not loaded before Phase 5; sync status and MT5 bridge entries appear in their phases.
- **Why:** the Phase 3 accept line (first sign-in creates one user, the second reuses it) needs a way to sign in, and delete account (CUJ 8) belongs with accounts; loading a file that does not exist until Phase 5 is a 404 (R27).
- **Evidence:** spec items 24, 25; `.paul/design/opendesign/vtradingview/01-signin-avatar-menu.html`, `04-delete-account.html`.

## D-014: Single-table SQL only (pgsqlite through sqlx)
- **Date:** 2026-11-23  **Plan:** 03 architect grounding  **Status:** decided (measured)
- **Decision:** every statement the server issues touches exactly one table and uses no `RETURNING`, join, subquery, cast, function or column default. Multi-table work is a sequence of single-table statements in one transaction. An upsert is `INSERT ... ON CONFLICT DO NOTHING` followed by a `SELECT` by key. A uniqueness conflict is SQLSTATE `23505` (Postgres) or `42000` whose message contains `UNIQUE constraint failed` (pgsqlite). The test suite runs on pgsqlite and Postgres 17.
- **Why:** a sqlx 0.8 probe against the owner's `~/.local/bin/pgsqlite` 0.0.22 decoded single-table reads correctly but returned `RETURNING`, join and subquery columns typed TEXT with binary payloads (undecodable), and casts gave a protocol error; the fork build fixes only plain `RETURNING`. Rejected: requiring the fork (owner uses the installed binary), decoding everything as text (casts break too).
- **Evidence:** `.paul/phases/03-auth/03-GROUNDING.md` (probe table); spec "Phase 3 facts and decisions".

## D-015: Any desktop exchange attempt burns the code
- **Date:** 2026-11-24  **Plan:** 03 architect  **Status:** decided (written default at plan time; owner may override)
- **Decision:** `POST /auth/desktop/exchange` deletes the code row before any check (`rows_affected == 1` wins), so a wrong `desktop_key`, an expired code and a reuse all spend it; the app restarts the flow.
- **Why:** stops guessing `desktop_key` against a known code; the one-time property rests on the measured exact `rows_affected`. Cost accepted: whoever holds a leaked code can cancel a real sign-in (not complete it).
- **Evidence:** `.paul/phases/03-auth/03-ARCHITECTURE.md` (Open questions; statement S13); grounding Addendum.

## D-016: No database readiness on /api/health in Phase 3
- **Date:** 2026-11-24  **Plan:** 03 architect  **Status:** decided (written default)
- **Decision:** `/api/health` keeps its Phase 1 parity body. With auth on, start-up already fails when the database cannot connect or migrate (`Db::open`); a readiness field arrives with the Phase 4 deploy.
- **Why:** keeps the parity contract unchanged and puts readiness where the deploy needs it.
- **Evidence:** `03-ARCHITECTURE.md` Open questions; spec item 20 (superseded).

## D-017: /auth/me carries the public Google client id
- **Date:** 2026-11-24  **Plan:** 03 architect  **Status:** decided (written default)
- **Decision:** with auth on, every `/auth/me` body adds `"client_id": "<GOOGLE_CLIENT_ID>"` (additive to the R4 shape); `hub/auth.js` reads it there. No templating of static HTML.
- **Why:** the client id is public by design (it is in every Google sign-in page), and one request gives the page both the mode and the id.
- **Evidence:** `03-ARCHITECTURE.md` Open questions; spec "Phase 3 facts and decisions" (Routes).

## D-018: Every pool checks a connection before handing it out (pgsqlite desync after an error)
- **Date:** 2026-11-24  **Plan:** 03-01 task 1  **Status:** decided (measured)
- **Decision:** every pool in hub-core and its tests is built from `db::pool_options()`, whose `before_acquire` runs `SELECT 1` and closes any connection that does not answer 1. Inside a transaction, the first error ends the transaction (propagate it; never issue another statement on that transaction).
- **Why:** on pgsqlite (owner's 0.0.22 and the fork) a statement error leaves the connection's replies one message late: the next re-executed INSERT reported success with 0 rows and wrote nothing, and later statements failed with "expected ParseComplete, got ReadyForQuery". Closing the poisoned connection rolls back its transaction and leaves other connections writable (measured). `SELECT 1` always fails on a poisoned connection (measured). Postgres pays one loopback round trip per acquire. The root fix is being made in `~/project/pgsqlite` (owner's request); the guard stays as defence.
- **Evidence:** `tests/db_engines.rs::a_failed_statement_does_not_poison_later_writes`; `.paul/phases/03-auth/03-GROUNDING.md` Addendum 2.

## D-022: The TTFC beacon posts with fetch keepalive only, and pages capture no t0
- **Date:** 2026-11-25  **Plan:** 04-02 tasks 3-4  **Status:** decided (deviation adopted at qualify)
- **Decision:** `hub/ttfc.js` posts with `fetch(..., {keepalive: true})` as the single path (the plan sketched `sendBeacon || fetch`), and the ms value is `Math.round(performance.now())` read inside ttfc.js at the first live bar; the engine pages capture no `t0`.
- **Why:** `sendBeacon` sends `text/plain`, which `net::post_guard` answers 415, so the beacon would never be accepted; one fetch path keeps the guard's rules exactly satisfied. `performance.now()` is navigation-start-relative, so measuring inside ttfc.js is equivalent and leaves the hashed inline script blocks smaller (the CSP hashes fewer edited bytes).
- **Evidence:** `.paul/runs/check-0402-final.log` — every observed beacon was `application/json` and answered 204; the m1 mutation still blocks a rogue inline block.

## D-023: DEBT-015 is discharged by wiring, not by one lucky run
- **Date:** 2026-11-25  **Plan:** 04-02 task 5  **Status:** decided
- **Decision:** `check-0303.mjs` runs inside `check-0402.sh` (opt-in `CHECK_0303=1`) against the VTV_TEST_IDP=1 server the harness already starts; the status comes from `PIPESTATUS[0]` so `tee` cannot mask a failure. The CI-headless half stays open, narrowed into DEBT-017's runner work (04-03).
- **Why:** the old check named a bare `node check-0303.mjs`, which cannot pass from a clean shell (no live server, no kit node_modules) — a check that cannot run is not a check (L-030). This run recorded 66/66 under the CSP this plan shipped.
- **Evidence:** `.paul/DEBT.toml` DEBT-015 (`status = "done"`, check = `text_present` on `CHECK_0303=1`); `.paul/runs/check-0402-final.log`.

## D-019: The header control is sticky to the bar's right edge
- **Date:** 2026-11-24  **Plan:** 03-03 (header control, `#authSlot`)  **Status:** decided
- **Decision:** the sign-in/avatar control sticks to the right edge of the top bar, because the bar itself scrolls sideways at 1280 px.
- **Why:** a control anchored inside the scrolling content drifts off-screen; sticking it to the bar edge keeps sign-in reachable at every width (checked at 320/768/1280 in the 03-03 browser check).
- **Evidence:** `hub/hub.html` `#authSlot`; the 03-03 responsive pass.

## D-020: Delete confirmation uses the native `<dialog>` with `showModal()`
- **Date:** 2026-11-24  **Plan:** 03-03 task (delete dialog)  **Status:** decided
- **Decision:** the delete-account confirmation is a native `<dialog>` opened with `showModal()`; the browser owns the focus trap and the Escape key.
- **Why:** the modern-web-guidance dialog rule: the platform's modal already traps focus, closes on Esc, and dims the background, so hand-rolled trap code would be more code for worse behaviour.
- **Evidence:** `hub/auth.js`; the 03-03 dialog checks (focus starts on Cancel, stays inside, Escape cancels).

## D-021: The test-idp build serves `/__test-idp/*` behind three guards
- **Date:** 2026-11-24  **Plan:** 03-02/03-03 (test identity provider)  **Status:** decided
- **Decision:** `VTV_TEST_IDP=1` makes a `--features test-idp` debug build serve the in-process Idp under `/__test-idp/*`; a release build can never reach it.
- **Why:** the browser check needs a real sign-in flow without touching Google. Three guards: the cargo feature (never in a release build), `cfg!(debug_assertions)` (a release binary refuses even if the feature leaked in), and the env var. CI asserts the feature tree stays out of release (`cargo tree` step).
- **Evidence:** `crates/hub-core/src/lib.rs` `test_idp_deps()`; the ci.yml "release build excludes test-idp" step.

## D-024: The per-value sync cap is 256 KiB, named for the measured ceiling
- **Date:** 2026-11-25  **Plan:** 05-01  **Status:** decided (owner approved the phase 05 plan set, 2026-11-25)
- **Decision:** `VALUE_MAX = 262_144` bytes (256 KiB) per synced value, not the spec's 1 MB. The constant is named for the number that actually binds.
- **Why:** the gate (`routes/gate.rs`) refuses any request that declares a body over 300,000 bytes before a handler runs, so a 1 MB value was unreachable as specified; and JSON escaping can grow a value further. A per-route gate exemption would be the restructuring phase 05 is forbidden from doing. The spec's "1 MB" was a hypothesis; the gate is the measurement.
- **Evidence:** `crates/hub-core/src/routes/gate.rs:120-133` (the 300,000-byte refusal); both 05-01 architect candidates found it independently; `.paul/phases/05-sync/05-ARCHITECTURE.md` decision 1.

## D-025: sync.js is not added to hub/index.html
- **Date:** 2026-11-25  **Plan:** 05-03  **Status:** decided (owner approved the phase 05 plan set, 2026-11-25)
- **Decision:** spec item 41's page list is amended: `hub/index.html` gets no `sync.js`. It is a 10-line redirect stub with no chart, no writers and a `meta refresh` to `hub.html`; a wrapper there would sync nothing and cost a request on the redirect.
- **Why:** the sync layer's job is intercepting writes; a page with no writers gains nothing. The measured key table shows every allow-listed key is written from `hub.html` or one of the three engine pages.
- **Evidence:** `hub/index.html` (10 lines, meta refresh, zero `localStorage` use); `.paul/phases/05-sync/05-03-PLAN.md` Facts; `.paul/phases/05-sync/05-ARCHITECTURE.md` decision 9.
