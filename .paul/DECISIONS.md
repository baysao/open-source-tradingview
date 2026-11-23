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
