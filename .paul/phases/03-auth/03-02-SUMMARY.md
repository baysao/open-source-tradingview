---
phase: 03-auth
plan: 02
status: COMPLETE
completed: 2026-11-24
commits: 3cf8721 (code and suites), 16787d5 (AGENTS.md, DEBT-012)
---

# 03-02 SUMMARY: Google sign-in, sessions, the gate, every auth route

## What shipped

Hosted mode stands end to end. `AppState::start` opens and migrates the account database,
builds the `Auth` service with the production Idp, and `app(state)` serves the route tables
from `routes/table.rs` behind the area gate in `routes/gate.rs`. The flows (One Tap, the
button flow with PKCE, the desktop code handoff, exchange, logout, delete, the hourly sweep)
answer over HTTP, and every one was proven through the app on a real engine.

| Piece | Where |
|---|---|
| Pure rules: token verification, PKCE ticket, session slide, admin, exchange decision | `crates/hub-core/src/auth/rules.rs` |
| Sealed secrets (no Debug/Clone on raw ids), hashes, HKDF keys | `crates/hub-core/src/auth/ids.rs` |
| The 14 statements, private, one home | `crates/hub-core/src/auth/store.rs` |
| The service: flows as statement sequences, cookie format | `crates/hub-core/src/auth/mod.rs` |
| Google client (JWKS cache, token POST) and the test Idp | `auth/google.rs`, `auth/test_idp.rs` |
| The gate (Origin, JSON, size, admin-or-404, slide) | `crates/hub-core/src/routes/gate.rs` |
| Route lists as data, mounted by mode | `crates/hub-core/src/routes/table.rs` |
| Handlers (thin: parse, one Auth call, render) | `routes/auth.rs`, `routes/me.rs` |
| `AppState::start`, `build_with_auth`, hourly sweep, fallbacks | `crates/hub-core/src/lib.rs` |
| Suites: auth 14, admin 1, gate 4 | `crates/hub-core/tests/{auth,admin,gate}.rs`, `tests/common/hosted.rs` |

## AC results

| AC | Result | Evidence |
|---|---|---|
| AC-1 tokens verified exactly (pure tables) | PASS | `rules.rs` unit tables: aud, iss, exp == now, signature, HS256/none, missing claims, nonce, JWKS TTL and refetch, slide edges, ticket MAC and expiry |
| AC-2 first sign-in creates one user, second reuses it, both engines | PASS on pgsqlite | `auth::first_sign_in_creates_one_user_second_reuses`; concurrency: one user, one identity, both 200 |
| AC-3 button flow and the oauth cookie | PASS | `button_flow_sets_pkce_and_lands_signed_in`, six failure landings; the HTTP IdP path proves the verifier round trip (`http_idp_path_round_trips_the_verifier`) |
| AC-4 desktop codes one-time and key-bound | PASS | `desktop_code_is_one_time_and_bound`, `desktop_exchange_burns_the_code_on_every_attempt` (wrong key burns, +61 s refused, concurrent exchange one 200) |
| AC-5 the gate and admin 404, table driven | PASS | `gate::origin_rules_over_every_hosted_non_get`, `admin::admin_routes_are_404_to_everyone_but_a_verified_admin` |
| AC-6 sessions slide, end, delete cascades | PASS | `session_slides_at_24h_and_expires_after_30_days_idle`, `logout_and_delete_cascade`, `sweep_removes_expired_sessions_and_their_codes` |
| AC-7 modes, logging, the shape rule | PASS | `gate::local_and_hosted_modes_answer_the_shape_table`, `auth::log_ring_holds_no_secret_after_full_flows`, shape test, start-up exit 1, feature tree 0 |
| AC-8 the checks can fail | PASS | 22 mutations: 21 make their named test fail; 1 documented as Postgres-only (see below) |

**Postgres:** not run locally (no server on this PC). Every suite is `engines!`-driven, so CI's
`db` job runs the same tests on Postgres 17 (`DEBT-005`, first green CI run).

## Deviations from the plan, each verified

1. **`routes/table.rs` gained `mount_routes`.** `app()` mounts the lists from it, and
   `route_specs` enumerates them; the plan named only the three list functions.
2. **The fallback answers JSON 404 under `/auth/` and `/admin/` as well as `/api/`.** The
   architecture's error table says so; `app()` originally only covered `/api/`.
3. **The test IdP was repaired in five places.** It signed with hex text, so no token it
   minted could ever verify; it minted no `aud`; the authorize redirect dropped the granted
   code and the state; `/token` expected JSON where the client posts a form; and the mount
   nested the router under its own prefix twice. Each was found by a failing suite, not by
   inspection.
4. **`AuthDeps::over_http` exists now.** The architecture listed it; it was missing, and AC-3's
   verifier proof needs it.
5. **The sign-in effect and the desktop callback are one transaction per attempt, up to
   three attempts with a 50/100 ms pause on a busy database.** The plan's "one transaction
   each" and the 03-01 handoff's "retry on `database is locked`" required it; the bare-
   statement version left an orphan user row when it lost the S3 race.
6. **S4 and S8 renumbered to sequential placeholders.** pgsqlite 0.0.22 mis-maps parameters
   when a placeholder number is out of text order: `UPDATE users SET email = $2 ... WHERE id
   = $1` matched 0 rows and reported `rows_affected` 0, while the same statement with
   sequential numbers matched and persisted. Measured with direct probes. DEBT-012 records
   the engine bug so the natural order can return once it is fixed.
7. **S4's `rows_affected` is not a signal.** On pgsqlite it reports 0 for an unchanged row;
   the sign-in effect checks S7 (the user exists) after the update, which also preserves the
   deleted-mid-sign-in retry.
8. **S11 moved inside the desktop transaction.** On the pool it violated the foreign key on
   a placeholder session the other connection could not see yet.
9. **The test harness feeds every ring through a fan-out layer.** `set_default` is
   thread-local, so a handler on another tokio thread never reached the ring and the
   log-ring test passed vacuously. Found by the AC-8 pass; the test also asserts the
   refusal lines are present, so it cannot go vacuous again.

## What the AC-8 pass changed in the tests

Five tests passed with their guard removed. Each is fixed, not waived:

- `session_check` at exactly `expires_at` had no unit case: added.
- The nonce rule had no test with a real signed token: added (`a_token_with_the_wrong_nonce_is_refused`).
- The log ring held no events at all: fan-out plus a presence assertion.
- A refusal line must carry no credential field, not merely a redacted one: asserted.
- The raw-session-id mutation pointed at the reader, not the writer: repointed at `SessionId::hash`.

One mutation is recorded as not observable on pgsqlite: S13's `rows_affected == 1` guard.
pgsqlite serialises writers, so the second exchange is refused by S12 before S13 runs. The
Postgres half runs in CI.

## Security review (phase override, /security-review)

| Surface | Finding | Disposition |
|---|---|---|
| CSRF (D-010) | No token; every non-GET in an area requires `Origin == PUBLIC_ORIGIN`, missing refused; cookies `__Host-`, `Secure`, `SameSite=Lax` | accepted per D-010; table-tested over every mounted route |
| Admin boundary | `/admin/*` resolves the viewer and answers 404 before the Origin rule | accepted; 404 proven for signed-out, member, unverified admin email, foreign Origin |
| Open redirect | The only `Location` values are the IdP URL from config, `/hub.html`, `/hub.html?signin=failed` and `vtradingview://auth?code=` | no user-controlled redirect |
| Secret logging | Refusals log the variant only; the ring redacts `token=`, `authorization`, 32+ hex and 40+ base64url runs; the e2e test asserts no credential field on a refusal line | accepted |
| Timing | Desktop key compared with `subtle::ConstantTimeEq`; the MAC with `verify_slice` | accepted |
| SQL injection | No `format!` in `store.rs`; every value bound | accepted |
| Cookie flags over plain HTTP | `__Host-`/`Secure` cookies need HTTPS: a browser on `http://127.0.0.1` drops them | expected; local dev uses local mode, hosted sign-in is exercised over the tunnel or TLS (Phase 04) |
| Rate limit | `/auth/*` and `/admin/*` share the `/api/*` bucket (20/s, burst 40) | accepted |
| DB outage | 503 `service unavailable` on every gated route, never a faked signed-out | accepted |

`needs_validation` items: none. No finding became a debt row.

## Real Google wiring (owner's client secret, never committed)

`.env` (gitignored) now carries `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET` (from the owner's
downloaded client JSON), a generated `VTV_SECRET_KEY`, `AUTH_ENABLED=true`,
`PUBLIC_ORIGIN=http://127.0.0.1:8770` and the pgsqlite `DATABASE_URL`.

Verified live: hosted start migrates the database, `/auth/me` returns the real client id, and
`/auth/google/start?client=web` redirects to `accounts.google.com` with PKCE S256 and the
sealed `__Host-vtv_oauth` cookie. The round trip itself is the owner's check (DEBT-010): it
needs two things this PC cannot supply on its own.

1. **Register `http://127.0.0.1:8770/auth/google/callback`** in Google Cloud console for that
   client (the downloaded file lists only `https://vtradingview.com`).
2. **Serve over HTTPS** for the browser to keep `__Host-` cookies: use the Cloudflare tunnel
   (Phase 04) or a local TLS terminator. Plain `http://127.0.0.1` is refused by design
   (hosted mode rejects a non-https origin unless the `test-idp` feature is on, which is how
   the redirect above was proven without touching Google's console).

## Next

03-03 (frontend sign-in, log panel gating, browser check) needs `npx -y
modern-web-guidance@latest *` allowlisted first (`DEBT-009`, blocks the Phase 03 exit).
`DEBT-011` (pgsqlite `--host`) also blocks that exit. The phase's `/reflect` runs at the
UNIFY of 03-03, the last plan of the phase.
