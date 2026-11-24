---
phase: 03-auth
type: Grounding (architect Phase A)
date: 2026-11-23
---

# Phase 03 grounding: database, Google sign-in, admin

Inputs for the Phase B design runners. Everything below is measured from the repository or the
running tools on 2026-11-23, not taken from the spec alone.

## What exists (crate `hub-core`, `crates/hub-core/src/`)

- `lib.rs`: `AppState` (every field a cheap clone): `cfg: Arc<Config>`, `catalog`, `bars`,
  `clock`, `feeds`, `logs: LogBus`, `orderflow`, private `limiter`, private `sources`.
  `AppState::build(cfg) -> Result<AppState, ConfigError>` does no network I/O; `build_with(cfg,
  endpoints)` is the test variant (no files written, L-008). `spawn_background()` returns a handle
  whose drop aborts the loops. `app(state) -> Router` is the only router: `/api/*` routes, `/ws`,
  `/api/logs` (GET), `/api/logs/journal` (POST), fallback (`/api/*` unknown gives 404
  `{"error":"not found"}`, else static files), layers `net::rate_limit` (token bucket 20/s, burst 40
  per client IP) and `net::security_headers` (nosniff, `Referrer-Policy: same-origin`; CSP is Phase 4).
- `config.rs`: `Config::from_env()` is the only env reader; `from_lookup(get)` for tests. Fields:
  `http_port` (8770), `static_dir`, `log_dir`, `cache_dir`, `public_origin` (default
  `http://127.0.0.1:<port>`), `auth: AuthMode` (only `Off`), `feed_max_symbols` (400).
  `ConfigError::{Invalid{key,value}, AuthNotAvailable}`. `AUTH_ENABLED=true` is refused today.
  `bind_addr()` is loopback by construction.
- `net.rs`: `ClientIp` extractor (`CF-Connecting-IP` trusted only from a loopback peer);
  `post_guard(cfg, headers, body_len)`: an `Origin`, when present, must be `http://localhost:<port>`,
  `http://127.0.0.1:<port>` or `PUBLIC_ORIGIN` (403 `forbidden origin`), content type must contain
  `application/json` (415 `JSON only`), body over 300,000 bytes is 413. A missing `Origin` is allowed
  (local tools). Python parity.
- `routes/mod.rs`: `json(status, body)`, `json_ok`, `api_error(status, msg)`; every API response has
  `Content-Type: application/json` and `Cache-Control: no-store`.
- `routes/logs.rs`: `logs` (GET `since`, `brief`) and `journal` (POST stub behind `post_guard`).
- `routes/ws.rs`: the websocket upgrade (anonymous today).
- `crates/hub-server/src/main.rs`: `.env` files, `Config::from_env`, `AppState::build`, tracing
  (stdout plus `RingLayer` into `LogBus`), bind, `spawn_background`, `axum::serve` with
  `ConnectInfo`, graceful ctrl-c.
- Workspace deps (Cargo.toml): axum 0.8 (ws), tokio full, tower 0.5, tower-http 0.6, reqwest 0.12
  rustls, serde, serde_json (float_roundtrip; tests add preserve_order), tracing, lru, regex, dotenvy,
  zip. No sqlx, jsonwebtoken, rand, sha2, hmac, hkdf, base64, uuid, chrono yet.
- Frontend: `hub/hub.html` (376 lines; `theme.js` first, inline app script, `log_panel.js` last);
  `hub/log_panel.js` polls `/api/logs?brief=1` every 5 s for every visitor and `/api/logs?since=`
  every 1.5 s when open; journal POST to `/api/logs/journal`. Mockups:
  `.paul/design/opendesign/vtradingview/01-signin-avatar-menu.html`, `04-delete-account.html`,
  `05-admin-log-panel.html`, `06-desktop-signin-waiting.html`.
- Tests: `crates/hub-core/tests/*.rs` integration suites against `app(state)` with fakes; gates are
  `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`.
  Clippy `disallowed_methods` is denied inside `orderflow` only.

## The spec (`.paul/design/specs/vtradingview.md`)

The "Phase 3 facts and decisions" block (before item 19) is authoritative and wins over items 19 to
29 where they disagree (items 20, 21, 23, 24, 25, 26 still name `sqlx::migrate!`, `csrf.rs`, the CSRF
header, `sync.js`, `/admin/restart`, `/admin/metrics`; all superseded). Read the block and items 19 to
29 in full, plus the "Authentication" section (around lines 270 to 312) for the flows: web One Tap,
web button (PKCE), desktop (system browser, one-time code bound to `desktop_key`, deep link
`vtradingview://auth?code=`, POST `/auth/desktop/exchange`), sign-in effect, requests, admin.

Decisions to obey (`.paul/DECISIONS.md`): D-001 (local mode: no DB, no sign-in, 127.0.0.1),
D-007 (limiter), D-010 (CSRF by `Origin` + `SameSite=Lax`, no token), D-011 (own migration runner,
plain DDL, ids and times from Rust), D-012 (logs public in local mode, admin-only with auth on),
D-013 (sign-in UI in Phase 3, `sync.js` from Phase 5).

## Measured: pgsqlite through sqlx 0.8 (D-014, new)

Probe: a throwaway sqlx 0.8 program (postgres driver, uuid + chrono) against the owner's
`~/.local/bin/pgsqlite` 0.0.22, and against a build of the `fix/insert-select-returning-timestamptz`
fork. Results on the owner's binary (the dev and CI target):

| Statement | Result |
|---|---|
| `CREATE TABLE` with uuid, text, boolean, bytea, bigint, timestamptz, composite PK, FK cascade | works |
| `INSERT` with typed binds (Uuid, bool, DateTime<Utc>, Vec<u8>, Option<String>) | works |
| single-table `SELECT` decoding uuid, bool, timestamptz, bytea, int8 | works, correct types |
| `UPDATE ... WHERE` with binds, `DELETE ... WHERE` | works; `ON DELETE CASCADE` removes children |
| `count(*)` from one table | works (INT8) |
| transaction `begin` / `rollback` | works; rolled-back rows are gone |
| `sqlx::raw_sql` multi-statement file | works |
| `INSERT ... RETURNING col` | broken: column typed TEXT with binary payload (fork: fixed for plain columns) |
| `RETURNING CAST(col AS text)` | broken (fork: returns no row) |
| `JOIN` across two tables | uuid and bool columns come back typed TEXT, undecodable; with casts: protocol error |
| `SELECT (SELECT count(*) ...), (SELECT ...)` | wrong type (INT4 label, binary payload) |
| unique violation | SQLSTATE `42000`, message `UNIQUE constraint failed: <table>.<cols>` (Postgres: `23505`) |
| any error | leaves the pool usable |

Earlier `psql` probes: no `decode()`, no `pg_advisory_lock`, `DEFAULT now()` becomes invalid SQLite
on the owner's binary. Composite `UNIQUE`, FK enforcement and `ON CONFLICT ... DO NOTHING` work.

**Consequence (binding):** every SQL statement the server issues touches exactly one table, has no
`RETURNING`, no join, no subquery, no cast, no function call, no column default; uniqueness
conflicts are detected by SQLSTATE `23505` or `42000` with `UNIQUE constraint failed` in the
message. Multi-table work is a sequence of single-table statements inside one transaction. The same
SQL runs on Postgres 17 (CI); the suite runs on both engines.

## Constraints from the project

- No emoji, no em or en dash in any file. No secrets in files or logs. Read-only market data.
- Local only: do not expose ports 8770, 8765, 8797. Release builds bind 127.0.0.1 (the tunnel ingresses).
- No native `<select>`; no one-sided coloured border; tokens only (frontend, 03-03).
- Learnings to inherit: L-003 (mutation-test every new check), L-008 (tests write no files),
  L-016 (no sockets under the paused clock; inject in-memory sources), L-001 (`tokio::time::Instant`).
- Tests must not need Google: a `test-idp` cargo feature provides a local RS256 signer and JWKS,
  and the IdP URLs come from config only in that feature.
- Plan boundaries asked by the owner: 03-01 config, db, migrations; 03-02 auth and routes;
  03-03 frontend sign-in, log panel, browser check.

## Addendum: second probe (after runner A and B named four unmeasured behaviors)

Owner's `~/.local/bin/pgsqlite` 0.0.22 through sqlx 0.8:

| Behavior | Result |
|---|---|
| `DELETE ... WHERE` `rows_affected` | exact: 1 on a hit, 0 on a miss, 2 on two rows (the one-time desktop code may rest on it) |
| `INSERT ... ON CONFLICT (k) DO NOTHING` | `rows_affected` 1 for a new row, 0 for a duplicate |
| `UPDATE` on no row | `rows_affected` 0 |
| `CREATE TABLE IF NOT EXISTS` run twice | FAILS: `table t already exists` (42000); the owner's binary drops `IF NOT EXISTS`. The runner detects `schema_migrations` by selecting from it (an error containing `no such table` or SQLSTATE `42P01` means absent), never by `IF NOT EXISTS` |
| `--in-memory` with a pool of 4 | UNUSABLE: each connection has its own empty database (`no such table` right after a create) |
| a database file with a pool of 4 | all connections see the same rows |
| socket file | none seen in `/tmp` on Windows |

Consequence for tests: a pgsqlite server serves one database file, so isolation is per server or
per schema reset. Two workable shapes: (a) one pgsqlite process per test, on a free port, over a fresh
file in the OS temp directory (outside the repo, deleted when the guard drops); (b) one process per test
binary, with tests serialized and the schema dropped and re-migrated between them. 03-01 picks one and
measures its start-up cost. A test pool of one connection would make `--in-memory` usable but hides
concurrency, so it is not used for the race tests.

## Addendum 2: found while applying 03-01 (2026-11-24)

| Behavior | Result | Handling |
|---|---|---|
| a statement error on a pooled connection | the connection's replies arrive one message late from then on: a re-executed cached INSERT reports `Ok(0)` and writes nothing; later statements fail with "expected ParseComplete, got ReadyForQuery". Same on the owner's 0.0.22 and the fork; statement cache off does not help | D-018: `db::pool_options()` closes any connection that fails `SELECT 1` before handing it out; closing rolls back its transaction (measured) |
| `timestamptz` written as a text literal | stored as text; a binary decode later fails ("Value Buffer exceeds 8 bytes") | nothing writes times as literals; tests bind values too |
| listen address | pgsqlite binds `0.0.0.0` with no password and has no listen-address option | DEBT-011 (blocks phase 03 exit); fix requested in `~/project/pgsqlite` |
| two write transactions on one file | the second waits about 5.5 s, then fails `42000 database is locked` | 03-02 treats it as retryable in the sign-in effect |
| start-up per test | 0.52 s mean (10 runs); `Db::open` with 0001 on a fresh file 0.18 s | per-test isolation kept (option a) |
