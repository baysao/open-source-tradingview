---
phase: 03-auth
plan: 02
subsystem: auth
tags: [axum, google-oauth2, pkce, sessions, csrf, admin-gate, pgsqlite, postgres, sqlx]

requires:
  - phase: 03-01
    provides: Db (connected and migrated), migration 0001, hosted config (AuthSettings), engine harness (TestDb, engines!), shape rule
  - phase: 01
    provides: app(), AppState, net::post_guard/rate_limit, routes::logs

provides:
  - auth module (Auth service, pure rules, sealed ids, private store, Google client, test Idp)
  - the area gate (Origin, JSON, size, admin-or-404, session slide)
  - route tables as data (shared/local/hosted) plus route_specs
  - AppState::start (hosted), build_with_auth (tests), hourly sweep
  - tests/auth.rs (14), tests/admin.rs (1), tests/gate.rs (4), tests/common/hosted.rs

affects: [03-03 frontend sign-in, 04 deploy, 05 sync, 06 desktop, 07 mt5 bridges]

tech-stack:
  added: [jsonwebtoken 9, hmac 0.12, hkdf 0.12, subtle 2, rsa 0.9 (test-idp only)]
  patterns:
    - one deep Auth service, thin handlers
    - the gate is one middleware keyed by area(path); route lists are data the tests enumerate
    - SQL lives only in auth/store.rs, proved by a shape test
    - injected clock (AuthClock::Manual), never a paused one

key-files:
  created:
    - crates/hub-core/src/auth/mod.rs
    - crates/hub-core/src/auth/rules.rs
    - crates/hub-core/src/auth/ids.rs
    - crates/hub-core/src/auth/store.rs
    - crates/hub-core/src/auth/google.rs
    - crates/hub-core/src/auth/test_idp.rs
    - crates/hub-core/src/routes/gate.rs
    - crates/hub-core/src/routes/table.rs
    - crates/hub-core/src/routes/auth.rs
    - crates/hub-core/src/routes/me.rs
    - crates/hub-core/tests/auth.rs
    - crates/hub-core/tests/admin.rs
    - crates/hub-core/tests/gate.rs
    - crates/hub-core/tests/common/hosted.rs
  modified:
    - crates/hub-core/src/lib.rs
    - crates/hub-core/src/net.rs
    - crates/hub-core/src/db.rs
    - crates/hub-core/src/config.rs
    - crates/hub-core/Cargo.toml
    - crates/hub-server/src/main.rs

key-decisions:
  - "The sign-in effect is one transaction per attempt, up to three, with a 50/100 ms pause on a busy database"
  - "S4 and S8 use sequential placeholders: pgsqlite 0.0.22 mis-maps out-of-order numbers (DEBT-012)"
  - "S4's rows_affected is not a signal on pgsqlite; S7 decides whether the user row survived"
  - "The test IdP carries its own state and mounts as a service (nest_service), not a second prefix"

patterns-established:
  - "A test that scans a ring must first prove the ring has events (the fan-out layer)"
  - "Every mutation in the architecture table is run: apply, prove a named test fails, restore, prove it passes"

duration: about 6 h across two sessions (03-02 tasks 1 and 2 on 2026-11-24 earlier; task 3 this session)
started: 2026-11-24T08:00:00+07:00
completed: 2026-11-24T14:44:10+07:00
description: "Google sign-in, sessions, the Origin and admin gate, and every auth route, proven on pgsqlite through app(state)"
type: Summary
about: "vtradingview"
---

# Phase 03 Plan 02: Google sign-in and routes Summary

**Hosted mode stands end to end: `AppState::start` opens and migrates the account database, the area gate guards every hosted route, and One Tap, PKCE, desktop codes, sessions, logout, delete and the hourly sweep all answer over HTTP on a real engine.**

## Performance

| Metric | Value |
|--------|-------|
| Duration | about 6 h across two sessions |
| Started | 2026-11-24 (tasks 1 and 2, earlier session) |
| Completed | 2026-11-24T14:44:10+07:00 |
| Tasks | 3 completed |
| Files modified | 17 in task 3 |

## Acceptance Criteria Results

| Criterion | Status | Notes |
|-----------|--------|-------|
| AC-1: Tokens are verified exactly (pure tables) | Pass | aud, iss, exp == now, signature, HS256, alg none, missing sub/email, nonce mismatch; JWKS TTL clamp and 61 s refetch; slide edges; ticket MAC and expiry |
| AC-2: Sign-in creates one user, on both engines | Pass on pgsqlite | second sign-in reuses the row and moves `last_login_at`; two concurrent first sign-ins give one user, one identity, both 200; nine forged credentials 401 with no row |
| AC-3: The web button flow and the oauth cookie | Pass | `S256` challenge, `__Host-vtv_oauth` Max-Age 600, landing `/hub.html`; six failure landings (tampered MAC, expired, wrong state, absent cookie, nonce, access_denied); the HTTP IdP path proves the verifier round trip |
| AC-4: Desktop codes are one-time and bound to the key (D-015) | Pass | deep link `vtradingview://auth?code=`; no session cookie handed out; +61 s refused; wrong key burns the code; second exchange 401; two concurrent exchanges give one 200 |
| AC-5: The gate (D-010) and the admin 404, table driven | Pass | every hosted non-GET refuses a missing and a foreign Origin and passes PUBLIC_ORIGIN; GET `/auth/me` needs none; 415 and 413; every route in an area; `/admin/*` 404 to signed-out, member, unverified admin; admin gets the logs body and journal 200 |
| AC-6: Sessions slide, end, and delete cascades | Pass | +23 h no cookie and unchanged row; +24 h 1 s one cookie and moved row; 30 days idle signed out; logout 204 twice; delete 204 with the tables cleared; sweep removes sessions and codes |
| AC-7: Modes, logging and the shape rule | Pass | local `/auth/me` `{"auth":false}` and hosted routes 404; hosted `/api/logs*` 404; no credential field on any refusal line; shape test; start-up exit 1 naming `VTV_SECRET_KEY`; feature tree 0 |
| AC-8: The checks can fail | Pass | 22 mutations: 22 make their named test fail, restore passes |

**Postgres:** not run locally (no server on this PC). Every suite is `engines!`-driven, so the CI
`db` job runs the same tests on Postgres 17 (`DEBT-005`).

## Accomplishments

- The whole hosted sign-in surface ships behind one gate, with the route lists as data so a
  route mounted outside an area fails a test instead of opening a door.
- Every flow is proven through `app(state)` on a real engine with a manual clock: no in-memory
  store to drift, no paused clock with sockets.
- Five blind tests were found and fixed by running the architecture's own mutation table; the
  suite that had been passing vacuously (the log ring) now cannot.

## Task Commits

| Task | Commit | Type | Description |
|------|--------|------|-------------|
| Task 1: pure rules, sealed ids, test IdP | `21c9a8f` | feat | rules tables, sealed newtypes, HKDF keys, TestIdp and ManualClock |
| Task 2: store, Google client, Auth service, AppState::start | `903e1b5`, `afc575e` | feat | S1-S14, JWKS cache, flows, start-up wiring, hourly sweep |
| Task 3: gate, route table, handlers, suites | `3cf8721` | test | the gate, tables, handlers, three suites, and the fixes below |
| Follow-ups | `16787d5`, `597e8c2` | docs, test | AGENTS.md skills, DEBT-012, ring-test assertion, busy backoff |

## Files Created/Modified

| File | Change | Purpose |
|------|--------|---------|
| `crates/hub-core/src/auth/rules.rs` | Created | Pure decisions: token verification, JWKS plan and TTL, session check, admin, exchange, sealed ticket |
| `crates/hub-core/src/auth/ids.rs` | Created | Sealed secrets and hashes; random values; HKDF keys |
| `crates/hub-core/src/auth/store.rs` | Created | The 14 statements in one private home, plus transaction variants |
| `crates/hub-core/src/auth/mod.rs` | Created | The flows as statement sequences, cookie format, the transaction retry |
| `crates/hub-core/src/auth/google.rs` | Created | JWKS fetch and cache, the token form POST |
| `crates/hub-core/src/auth/test_idp.rs` | Created | RSA signer, JWKS, authorize/token/mint router, manual clock |
| `crates/hub-core/src/routes/gate.rs` | Created | The one place the Origin, JSON, size and admin rules live |
| `crates/hub-core/src/routes/table.rs` | Created | Route lists by mode, mounted by `app()`, enumerated by tests |
| `crates/hub-core/src/routes/auth.rs`, `me.rs` | Created | Thin handlers: parse, one Auth call, render |
| `crates/hub-core/src/lib.rs` | Modified | `AppState::start`, `build_with_auth`, sweep task, table mount, fallback prefixes |
| `crates/hub-core/src/net.rs` | Modified | Limiter covers `/auth/` and `/admin/` |
| `crates/hub-core/src/db.rs` | Modified | `is_busy` classifier (D-018) |
| `crates/hub-core/tests/{auth,admin,gate}.rs`, `tests/common/hosted.rs` | Created | 19 tests over the app on both engines |

## Decisions Made

| Decision | Rationale | Impact |
|----------|-----------|--------|
| One transaction per sign-in attempt, up to three, with a short pause on a busy database | pgsqlite serialises writers and reports `database is locked`; bare statements left an orphan user row when S3 lost a race | A racer always converges; a busy database answers 503 rather than signed-out |
| S4 and S8 renumbered to sequential placeholders | pgsqlite 0.0.22 mis-maps parameters whose numbers are out of text order (measured: 0 rows, no error) | The workaround is local; DEBT-012 tracks the engine fix |
| S4's `rows_affected` is not a signal | pgsqlite reports 0 for an unchanged row | S7 decides; the deleted-mid-sign-in retry is preserved |
| The test IdP owns its state and mounts with `nest_service` | A second prefix and a host-state dependency both broke the HTTP path | 03-03's browser check can nest the same router |
| The refusal log carries the variant only | Verified by a test that rejects any `token=` or `credential=` field, even a redacted one | The audit trail stays useful and leak-free |

## Deviations from Plan

### Summary

| Type | Count | Impact |
|------|-------|--------|
| Auto-fixed | 9 | All essential: three were panics or silent auth failures |
| Scope additions | 2 | A `mount_routes` helper and `AuthDeps::over_http`, both named by the architecture |
| Deferred | 1 | DEBT-012 (the pgsqlite parameter-order bug) |

### Auto-fixed Issues

1. **Local mode panicked.** `app()` mounted `/api/logs` from the table *and* explicitly; axum
   panics on duplicate routes. The explicit mounts are gone. Files: `lib.rs`.
2. **The fallback under-answered.** JSON 404 covered `/api/` only, while the error table names
   `/auth/` and `/admin/` too. Files: `lib.rs`.
3. **`AuthDeps::over_http` was missing**, so AC-3's verifier proof had no path. Added.
4. **The test IdP could not sign a verifiable token.** `Signature::to_string()` returns hex
   text; jsonwebtoken wants raw bytes. Also: no `aud`, the granted code and state were dropped
   from the redirect, and `/token` expected JSON at a form endpoint. Files: `test_idp.rs`,
   `auth/mod.rs`.
5. **The sign-in effect had no transaction.** See the decision table. Files: `auth/mod.rs`,
   `store.rs`.
6. **S4/S8 parameter order** (pgsqlite bug). Files: `store.rs`.
7. **S11 ran outside the desktop transaction**, violating the FK on a placeholder session the
   other connection could not see. Files: `auth/mod.rs`.
8. **The log-ring test proved nothing**: `set_default` is thread-local, so a handler on another
   tokio thread never reached the ring. A fan-out layer now feeds every harness ring, and the
   test asserts the refusal lines are present.
9. **Four more blind tests**, found by the AC-8 pass and each fixed: the `expires_at == now`
   edge had no unit case; the nonce rule had no test with a real signed token; a refusal line
   only had to be *redacted*, not clean; the raw-session-id mutation pointed at the reader.

### Deferred Items

| Debt row | What is deferred | Trigger | Check |
|----------|------------------|---------|-------|
| DEBT-012 | pgsqlite 0.0.22 mis-maps out-of-order UPDATE parameters; the engine fix belongs in `~/project/pgsqlite` | `event:pgsqlite-param-order-fix` | the installed engine reports a version newer than 0.0.22 |
| DEBT-005 (existing) | The first green CI run, which also runs the Postgres half | `event:first-push` | `gh run list` conclusion |
| DEBT-009, DEBT-011 (existing) | modern-web-guidance guides for 03-03; pgsqlite `--host` | `event:npx-allowed`, `event:pgsqlite-host-option` | both block the Phase 03 exit |

## Issues Encountered

| Issue | Resolution |
|-------|------------|
| The concurrency test failed one run in three (`database is locked`) | Three attempts with a 50/100 ms pause; stable over five consecutive runs |
| The disk filled during a mutation run and truncated `store.rs` to zero bytes | Recovered from `HEAD`, re-applied this session's changes, verified by the suites, then committed so it could not recur |
| The mutation harness itself had wrong anchors and silently mis-targeted two rules | Anchors made unique; a missed anchor is now reported, never skipped |

## Security Review

| Surface | Finding | Disposition |
|---------|---------|-------------|
| CSRF (D-010) | No token; every non-GET in an area requires the exact Origin; `__Host-`, `Secure`, `SameSite=Lax` | accepted; table-tested over every mounted route |
| Admin boundary | Admin-or-404 before the Origin rule (an outsider learns nothing) | accepted; four identities proven |
| Open redirect | Only fixed `Location` values and the configured IdP URL | none |
| Secret logging | Refusals log the variant only; the ring redacts `token=`, 32+ hex and 40+ base64url; the test rejects any credential field | accepted |
| Timing | Constant-time key compare (`subtle`), `verify_slice` for the MAC | accepted |
| Cookie flags over plain HTTP | `__Host-` needs HTTPS; a browser on `http://127.0.0.1` drops them | expected; Phase 04 tunnel or TLS |
| Rate limit, DB outage | `/auth/*` and `/admin/*` share the bucket; a database outage is 503, never a faked signed-out | accepted |

No `needs_validation` records; no finding became a debt row.

## Next Phase Readiness

**Ready:**
- The session and the admin flag every later phase keys on (`/auth/me`, `Extension<Viewer>`).
- Hosted route tables and the gate that 03-03's UI and 05's sync endpoints mount into.
- A test harness (`tests/common/hosted.rs`) with the test IdP, manual clock and both engines.

**Concerns:**
- Postgres runs only in CI until `DEBT-005` clears; pgsqlite's quirks are worked around, and
  DEBT-012 tracks the engine fix.
- The real Google round trip needs the loopback redirect URI registered and an HTTPS origin
  (`DEBT-010`, the owner's check).

**Blockers:**
- `DEBT-009` (npx allowlist for 03-03) and `DEBT-011` (pgsqlite `--host`) block the Phase 03 exit.

## Real Google wiring (owner's credentials, never committed)

`.env` (gitignored) carries `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, a generated
`VTV_SECRET_KEY`, `AUTH_ENABLED=true`, `PUBLIC_ORIGIN=http://127.0.0.1:8770` and the pgsqlite
`DATABASE_URL`. Verified live: hosted start migrated the database, `/auth/me` returned the real
client id, and `/auth/google/start?client=web` redirected to `accounts.google.com` with PKCE
S256 and the sealed ticket. The round trip needs `http://127.0.0.1:8770/auth/google/callback`
registered in the Google Cloud console and an HTTPS origin for the `__Host-` cookies.

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack)
- instruction edits proposed: none
- rsi: deferred to phase close (reflect: pstack)

---
*Built with PAUL Framework v1.4*
*Phase: 03-auth, Plan: 02*
*Completed: 2026-11-24*
