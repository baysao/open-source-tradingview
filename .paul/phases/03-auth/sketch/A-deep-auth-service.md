---
phase: 03-auth
type: Architecture candidate (architect Phase B)
candidate: A, one deep Auth service
date: 2026-11-23
---

# Candidate A: one deep `Auth` service

## Problem

Phase 03 adds a database, Google sign-in (One Tap, button with PKCE, desktop one-time codes), server
sessions and an admin gate to a server that today has no accounts and must keep a local mode with no
database (D-001). Four things make the shape non-obvious. (1) The database is pgsqlite in dev and CI
and Postgres 17 in CI: every statement must touch one table with no `RETURNING`, join, subquery,
cast, function or column default (D-014, measured), so every multi-table operation is a hand-ordered
sequence of single-table statements in one transaction, and the rule must live in exactly one place
or it will be broken by the first convenient join. (2) Security policy is spread across many request
shapes: the `Origin` rule for non-GET on `/auth/*`, `/api/me*`, `/admin/*` (D-010, a missing
`Origin` refused, unlike `net::post_guard`), the admin 404, the 24 h session slide that must also
re-set the cookie, the MAC'd `__Host-vtv_oauth` cookie, the JWKS kid-refetch limit. (3) Tests must
never call Google, must write no files (L-008), must not open sockets under a paused clock (L-016),
yet need to move time by 24 h, 30 days, 60 s and 1 min. (4) `app(state)` stays the only router and
`AppState::build` does no network I/O. The schema, routes, bodies and key derivation are fixed by the
spec's "Phase 3 facts and decisions" block, which this design follows wherever items 19 to 29 differ.

## Usage (caller's view)

README of `hub_core::auth`: "With `AUTH_ENABLED=true`, `AppState.auth` is `Some(Auth)`. `Auth` is the
whole sign-in system: give it HTTP facts (method, path, headers, a credential, a callback query), get
back domain results that already carry the `Set-Cookie` values and redirect targets. Nothing outside
`auth` sees SQL, hashes, cookie formats or Google's wire types."

```rust
// hub-server/src/main.rs (03-02): the only new lines
let state = AppState::build(cfg)?;                 // Auth built lazily: no I/O, as today
if let Some(auth) = &state.auth {
    auth.prepare().await?;                         // connect + migrate (fails start-up with reason)
}
let _bg = state.spawn_background();                // + auth.sweep_expired() hourly when Some

// routes/auth.rs (03-02): a whole handler, One Tap
pub async fn onetap(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = st.auth.as_ref().expect("mounted only with auth on");
    let Ok(req) = serde_json::from_slice::<OneTapBody>(&body) else { return bad_request() };
    match auth.sign_in_with_id_token(&req.credential, user_agent(&headers)).await {
        Ok(s) => s.cookie.attach(json_ok(&SignedInBody::from(&s.member))),
        Err(e) => e.into_response(),               // 401 {"error":"sign-in failed"}, reason logged
    }
}

// routes/auth.rs: the callback never answers JSON, only a redirect
pub async fn callback(State(st): State<AppState>, headers: HeaderMap,
                      Query(q): Query<CallbackQuery>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only with auth on");
    auth.finish_callback(&headers, q.into()).await.into_response()   // 302 + cookies, always
}

// routes/me.rs: DELETE /api/me; the gate already resolved the viewer
pub async fn delete_me(State(st): State<AppState>, Extension(v): Extension<Viewer>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only with auth on");
    match auth.delete_account(&v).await {
        Ok(clear) => clear.attach(StatusCode::NO_CONTENT.into_response()),
        Err(e) => e.into_response(),
    }
}

// tests/auth.rs (03-02): the whole flow with no Google and no socket to an IdP
let h = Harness::start(Engine::Pgsqlite).await;    // in-memory pgsqlite, TestIdp, ManualClock
let tok = h.idp.id_token(Claims::for_sub("g-1").email("a@x.io", true));
let r = h.post("/auth/google/onetap", json!({"credential": tok})).origin_ok().send().await;
assert_eq!(r.status(), 200);
h.clock.advance(Duration::from_secs(24 * 3600 + 1));
assert!(h.get("/auth/me").cookie(&r).send().await.sets_cookie("__Host-vtv_session"));
```

Public surface of `auth`: `Auth` and its eight operations, `AuthDeps`, `Viewer`, `Member`, `User`,
`SignedIn`, `CallbackOutcome`, `Admission`, `SetCookie`, `AuthError`, `Client`; with `test-idp`
also `test_idp::{TestIdp, Claims}` and `ManualClock`.

## Shape

### Module map

```text
crates/hub-core/
  migrations/0001_users.up.sql, 0001_users.down.sql   embedded with include_str!
  src/config.rs      AuthMode::{Off, Google(Arc<AuthSettings>)}, AuthSettings, Secret, ConfigError::Missing
  src/db.rs          Db (lazy PgPool), migration runner, is_unique_violation      (pub, no domain SQL)
  src/auth/mod.rs    Auth + operations + domain types + AuthError -> Response     (the deep module)
  src/auth/store.rs  every domain SQL statement; private; the only file with SQL text besides db.rs
  src/auth/google.rs Idp transport enum, JWKS cache, id-token verify, code exchange (private)
  src/auth/crypto.rs HKDF keys, random 32-byte ids, SHA-256, oauth cookie seal/open (private, pure)
  src/auth/test_idp.rs  #[cfg(feature = "test-idp")] RS256 signer, JWKS, token endpoint, router
  src/routes/auth.rs    gate middleware + /auth/* handlers (thin)
  src/routes/me.rs      DELETE /api/me (thin)
  src/lib.rs            AppState.auth, app(): two route tables chosen by state.auth.is_some()
```

A reader traces any flow in three files: `routes/auth.rs` (adapter), `auth/mod.rs` (the operation,
which reads as the step list), `auth/store.rs` (the statements).

### Data structures

```rust
// config.rs
pub enum AuthMode { Off, Google(Arc<AuthSettings>) }        // replaces `AuthMode::Off` only
impl AuthMode { pub fn is_local(&self) -> bool { todo!() } } // `exempt_loopback` uses this
pub struct AuthSettings {
    pub database_url: Secret,          // DATABASE_URL
    pub client_id: String,             // GOOGLE_CLIENT_ID
    pub client_secret: Secret,         // GOOGLE_CLIENT_SECRET
    pub secret_key: SecretKey,         // VTV_SECRET_KEY: base64url, decoded length >= 32
    pub public_origin: HttpsOrigin,    // PUBLIC_ORIGIN, https:// (test-idp: http loopback too)
    pub admin_emails: Vec<String>,     // ADMIN_EMAILS, trimmed, lowercased, empty by default
    pub idp: IdpEndpoints,             // Google's constants; GOOGLE_* env only under test-idp
}
pub struct Secret(String);             // Debug prints "<redacted>"; no Display
pub struct SecretKey(Vec<u8>);         // Debug redacted; constructed only by the parser
pub struct IdpEndpoints { pub issuers: Vec<String>, pub jwks_url: String,
                          pub token_url: String, pub auth_url: String }
pub enum ConfigError {
    Invalid { key: &'static str, value: String },  // value omitted for secret keys
    Missing { keys: Vec<&'static str> },           // every missing name at once
}                                                  // AuthNotAvailable removed in 03-02

// auth/mod.rs
#[derive(Clone)] pub struct Auth(Arc<Inner>);      // cheap clone, like every AppState field
struct Inner {
    db: Db, keys: Keys, idp: google::Idp, jwks: google::JwksCache,
    clock: AuthClock, settings: Arc<AuthSettings>,
}
struct Keys { oauth_mac: [u8; 32], _reserved: [u8; 32] }   // HKDF-SHA256, salt "vtv"
pub enum AuthClock { System, #[cfg(feature = "test-idp")] Manual(ManualClock) }
// now(): DateTime<Utc> truncated to microseconds, so a value read back equals the value written
// on both engines. JWKS max-age, 24 h slide, 30 d expiry, 60 s code, 600 s cookie all use it.

pub struct AuthDeps { idp: google::Idp, clock: AuthClock }
impl AuthDeps {
    pub fn production() -> AuthDeps { todo!() }                    // reqwest + System clock
    #[cfg(feature = "test-idp")]
    pub fn test(idp: &TestIdp, clock: &ManualClock) -> AuthDeps { todo!() }   // in memory
    #[cfg(feature = "test-idp")]
    pub fn http_with_clock(clock: &ManualClock) -> AuthDeps { todo!() }       // real reqwest path
}

#[derive(Clone, Debug, Serialize)]
pub struct User { pub id: Uuid, pub email: String, pub name: Option<String>, pub picture: Option<String> }
#[derive(Clone, Debug)]
pub struct Member { pub user: User, pub admin: bool, session: SessionHash }   // session private
#[derive(Clone, Debug)]
pub enum Viewer { Anonymous, Member(Member) }
struct SessionHash([u8; 32]);          // SHA-256 of the raw id; the raw id never leaves crypto+cookie
pub enum Client { Web, Desktop { key_hash: [u8; 32] } }   // parsed from ?client=&dk=

pub struct SetCookie(Vec<HeaderValue>);   // private constructor: only Auth writes cookies
impl SetCookie { pub fn attach(self, resp: Response) -> Response { todo!() } }
pub struct SignedIn { pub member: Member, pub cookie: SetCookie }
pub enum Admission {
    Pass { viewer: Viewer, refresh: Option<SetCookie> },   // refresh = the 24 h slide re-set
    Refuse(AuthError),
}
pub enum CallbackOutcome {                // IntoResponse: always 302, oauth cookie always cleared
    Web { cookies: SetCookie },                             // -> /hub.html
    Desktop { code: String, cookies: SetCookie },           // -> vtradingview://auth?code=
    Failed { cookies: SetCookie, reason: Refusal },         // -> /hub.html?signin=failed, logged
}
```

The oauth cookie payload (`crypto.rs`, private): `OauthTicket { state, nonce, verifier, client:
"web"|"desktop", dk_hash: Option<b64>, exp: i64 }`, serialized as base64url(JSON) `.` base64url(
HMAC-SHA256(oauth_mac, json_bytes)). `open()` verifies the MAC with `Mac::verify_slice` (constant
time) before parsing JSON, then checks `exp`.

### The operations (the whole public interface)

```rust
impl Auth {
    /// No I/O: lazy pool (PgPoolOptions::connect_lazy_with), keys derived, empty JWKS cache.
    pub fn new(settings: Arc<AuthSettings>, deps: AuthDeps) -> Result<Auth, ConfigError> { todo!() }
    /// Connects and runs the embedded migrations. main.rs and tests call it once.
    pub async fn prepare(&self) -> Result<(), DbError> { todo!() }

    /// The request gate (middleware). Pure prefix policy plus at most one session read:
    /// 1. path outside /auth/, /api/me, /admin/ -> Pass(Anonymous), no DB hit.
    /// 2. resolve the session cookie (S6, S7; S8 when last_seen_at is 24 h old -> refresh cookie).
    /// 3. /admin/* and not admin -> Refuse(NotFound)  (before the Origin rule: nothing leaks)
    /// 4. non-GET/HEAD and Origin != PUBLIC_ORIGIN (missing included) -> Refuse(ForbiddenOrigin)
    /// 5. non-GET with a body and no application/json content type -> Refuse(JsonOnly)
    pub async fn admit(&self, method: &Method, path: &str, headers: &HeaderMap) -> Admission { todo!() }

    /// One Tap: verify (RS256, kid, aud, iss, exp, sub, email) then the sign-in effect.
    pub async fn sign_in_with_id_token(&self, credential: &str, ua: Option<&str>)
        -> Result<SignedIn, AuthError> { todo!() }
    /// GET /auth/google/start: 302 target plus the sealed oauth cookie. No DB.
    pub fn begin_sign_in(&self, client: &str, dk: Option<&str>) -> Result<(Uri, SetCookie), AuthError> { todo!() }
    /// GET /auth/google/callback: open cookie, check state, exchange code with verifier, verify
    /// id token with nonce, sign-in effect; desktop also issues the one-time code. Never errors.
    pub async fn finish_callback(&self, headers: &HeaderMap, q: CallbackParams) -> CallbackOutcome { todo!() }
    /// POST /auth/desktop/exchange: burn the code, check expiry and key, rotate the placeholder
    /// session into a fresh one, return it with its cookie.
    pub async fn exchange_desktop_code(&self, code: &str, desktop_key: &str, ua: Option<&str>)
        -> Result<SignedIn, AuthError> { todo!() }
    /// Idempotent: deletes the viewer's session row if any; always returns the clearing cookie.
    pub async fn sign_out(&self, viewer: &Viewer) -> Result<SetCookie, AuthError> { todo!() }
    /// DELETE /api/me: one DELETE on users; identities, sessions, codes go by cascade.
    pub async fn delete_account(&self, viewer: &Viewer) -> Result<SetCookie, AuthError> { todo!() }
    /// Background, hourly: S14. Idempotent.
    pub async fn sweep_expired(&self) -> Result<u64, AuthError> { todo!() }
}
```

`/auth/me` needs no operation: the handler reads `Extension<Viewer>` the gate inserted and the gate
already attached any slide cookie. That is deliberate: the session is resolved once per request in
one place, and the slide cannot be forgotten by a new handler.

### Load-bearing decisions

1. **The gate is an `Auth` operation, mounted once** (`routes::auth::gate`, from_fn_with_state,
   inside `security_headers`, outside the routes). Origin rule, JSON rule, admin 404, session
   resolution and slide live in one function with one test table. A handler added under `/admin/`
   later is gated without doing anything (per encode-lessons-in-structure).
2. **Two route tables.** `app()` mounts `/api/logs*` only when `state.auth.is_none()`, and
   `/admin/logs*`, `/auth/google/*`, `/auth/desktop/exchange`, `/auth/logout`, `DELETE /api/me`
   only when `Some`; `/auth/me` in both (local returns `{"auth": false}`). The `/admin/logs*`
   handlers are the existing `routes::logs::{logs, journal}`. Routing derives from
   `state.auth.is_some()`; `AppState::build` sets it from `cfg.auth` in one constructor, so the two
   cannot disagree (single source of truth).
3. **Single-table SQL is a private module, not a convention.** `auth/store.rs` exposes typed
   functions taking `&mut PgConnection`; `auth/mod.rs` composes them in transactions. The rule has one
   home; a CI grep (`scripts` not needed: a unit test in `store.rs` scans its own SQL constants for
   ` JOIN `, `RETURNING`, `SELECT (`, `::`, `CAST(`, `now()`, `DEFAULT`) fails the build on a breach.
4. **Desktop code without a new key.** The schema binds a code to `session_id_hash`, but only the
   hash is stored, so the raw session id cannot be recovered at exchange. The callback therefore
   creates a placeholder session (expires in 60 s, raw id discarded at once, unusable by anyone) and
   the exchange burns the code, deletes the placeholder and creates the real session. No third HKDF
   key, schema unchanged, cascade still covers codes.
5. **Time is injected, not paused.** `AuthClock::Manual` moves 24 h in a test while pgsqlite and
   the optional HTTP IdP run on real sockets under real tokio time (L-016 holds).
6. **The IdP is an enum, not a trait object** (`Idp::Google(HttpIdp)` and, under `test-idp`,
   `Idp::Memory(TestIdp)`). Release builds contain one variant; `Memory` cannot exist there.

### Flows as statement sequences (all in `store.rs`, run inside one transaction per operation)

Sign-in effect (`claims` already verified; `now` from the clock; up to two attempts):
`S1` find identity; none -> `S2` insert user (UUID v7 from Rust), `S3` insert identity ON CONFLICT
DO NOTHING, `S1` again: if the found `user_id` is not ours a concurrent first sign-in won, roll back
(our user row goes) and retry, where `S1` now finds the winner. Found -> `S4` update profile and
`last_login_at`; zero rows (account deleted meanwhile) -> roll back, retry as new. Then `S5` session
(raw id 32 random bytes, row holds SHA-256), commit, `SetCookie` with base64url(raw id).

Resolve (gate): parse cookie (43 base64url chars -> 32 bytes, else Anonymous), `S6`; missing or
`expires_at <= now` -> Anonymous; `S7` user; admin = `email_verified` and
`admin_emails` contains `email.to_lowercase()`; `now - last_seen_at >= 24 h` -> `S8` (new `expires_at = now + 30 d`,
`last_seen_at = now`) and a refresh `SetCookie` with `Max-Age=2592000`. No transaction (reads plus
one idempotent update; two racing slides write near-equal values, harmless).

Desktop exchange: `S12` code; none -> Refused(CodeUnknown). `S13` delete it; `rows_affected != 1`
-> Refused (a concurrent exchange won: the one-time guard). Then the checks: `expires_at <= now` ->
Expired; `subtle::ConstantTimeEq` on SHA-256(desktop_key) -> KeyMismatch. Any refusal: `S9` the
placeholder, commit (the code stays burned), 401. Pass: `S6` placeholder for `user_id`, `S9` it,
`S7` user, `S5` fresh session, commit.

Callback desktop branch: sign-in effect with the placeholder session (`expires_at = now + 60 s`),
then `S11` code (32 random bytes, row holds hashes of code and desktop key), same transaction.

### SQL: migration 0001

```sql
-- 0001_users.up.sql
CREATE TABLE users (
    id uuid PRIMARY KEY,
    email text NOT NULL,
    email_verified boolean NOT NULL,
    name text NULL,
    picture text NULL,
    created_at timestamptz NOT NULL,
    last_login_at timestamptz NOT NULL
);
CREATE TABLE identities (
    provider text NOT NULL,
    subject text NOT NULL,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL,
    PRIMARY KEY (provider, subject)
);
CREATE INDEX identities_user_id ON identities (user_id);
CREATE TABLE sessions (
    id_hash bytea PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL,
    expires_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    user_agent text NULL
);
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE TABLE desktop_auth_codes (
    code_hash bytea PRIMARY KEY,
    desktop_key_hash bytea NOT NULL,
    session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);

-- 0001_users.down.sql
DROP TABLE desktop_auth_codes;
DROP TABLE sessions;
DROP TABLE identities;
DROP TABLE users;
```

### SQL: every statement the server issues

| Id | Statement | Used by |
|---|---|---|
| M1 | `CREATE TABLE IF NOT EXISTS schema_migrations (version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL, applied_at timestamptz NOT NULL)` | prepare |
| M2 | `SELECT version, name, checksum FROM schema_migrations ORDER BY version` | prepare |
| M3 | `INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)` | prepare, after the file, same tx |
| M4 | `DELETE FROM schema_migrations WHERE version = $1` | migrate_down (test-idp only) |
| S1 | `SELECT user_id FROM identities WHERE provider = $1 AND subject = $2` | sign-in |
| S2 | `INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)` | first sign-in |
| S3 | `INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (provider, subject) DO NOTHING` | first sign-in |
| S4 | `UPDATE users SET email = $2, email_verified = $3, name = $4, picture = $5, last_login_at = $6 WHERE id = $1` | later sign-in |
| S5 | `INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)` | sign-in, exchange |
| S6 | `SELECT user_id, expires_at, last_seen_at FROM sessions WHERE id_hash = $1` | gate, exchange |
| S7 | `SELECT id, email, email_verified, name, picture FROM users WHERE id = $1` | gate, exchange |
| S8 | `UPDATE sessions SET expires_at = $2, last_seen_at = $3 WHERE id_hash = $1` | gate slide |
| S9 | `DELETE FROM sessions WHERE id_hash = $1` | sign_out, exchange |
| S10 | `DELETE FROM users WHERE id = $1` | delete_account |
| S11 | `INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)` | desktop callback |
| S12 | `SELECT desktop_key_hash, session_id_hash, expires_at FROM desktop_auth_codes WHERE code_hash = $1` | exchange |
| S13 | `DELETE FROM desktop_auth_codes WHERE code_hash = $1` | exchange |
| S14 | `DELETE FROM sessions WHERE expires_at < $1` | sweep (codes by cascade) |

All through `sqlx::query(..).bind(..)` and `query_as::<_, (..)>` at runtime; no macros, no
`DATABASE_URL` at build time. Migration files run with `sqlx::raw_sql(up).execute(&mut *tx)`.

### db.rs (03-01)

```rust
#[derive(Clone)] pub struct Db { pool: PgPool }
pub struct Migration { pub version: i64, pub name: &'static str, pub up: &'static str, pub down: &'static str }
pub const MIGRATIONS: &[Migration] = &[/* 0001_users via include_str! */];
pub enum DbError { Connect(sqlx::Error), Checksum { version: i64 }, UnknownVersion { version: i64 },
                   Migration { version: i64, source: sqlx::Error }, Query(sqlx::Error) }
impl Db {
    pub fn lazy(url: &Secret) -> Result<Db, ConfigError> { todo!() }   // parse only, no I/O
    /// M1, M2; checksum = SHA-256(up). Applied with a different checksum -> Checksum; a recorded
    /// version this binary lacks -> UnknownVersion; each missing file: begin, raw_sql, M3, commit.
    /// A failure re-reads M2 once (a concurrent starter may have applied it) before giving up.
    pub async fn migrate_up(&self) -> Result<Vec<i64>, DbError> { todo!() }
    #[cfg(feature = "test-idp")] pub async fn migrate_down_all(&self) -> Result<(), DbError> { todo!() }
    #[cfg(feature = "test-idp")] pub async fn migrate_list(&self, list: &[Migration]) -> Result<Vec<i64>, DbError> { todo!() }
    pub(crate) fn pool(&self) -> &PgPool { todo!() }
}
/// 23505, or 42000 whose message contains "UNIQUE constraint failed" (pgsqlite).
pub fn is_unique_violation(e: &sqlx::Error) -> bool { todo!() }
```

A unit test lists `crates/hub-core/migrations/` (reading is allowed, L-008 forbids writing) and
asserts `MIGRATIONS` names every `NNNN_name.up.sql` with its `.down.sql`, in order.

### Error mapping

| Source | Status | Body | Logged |
|---|---|---|---|
| `Refuse(NotFound)`: `/admin/*` signed out, non-admin, unverified admin email | 404 | `{"error":"not found"}` | no |
| `Refuse(ForbiddenOrigin)`: non-GET on gated prefixes, Origin missing or not `PUBLIC_ORIGIN` | 403 | `{"error":"forbidden origin"}` | debug |
| `Refuse(JsonOnly)` | 415 | `{"error":"JSON only"}` | no |
| `AuthError::SignInFailed(Refusal)`: bad token, bad code, wrong key, expired, reused | 401 | `{"error":"sign-in failed"}` | warn, `Refusal` kind only, never the token |
| `AuthError::SignInRequired`: `DELETE /api/me` signed out | 401 | `{"error":"sign-in required"}` | no |
| `AuthError::BadRequest`: unparsable JSON body, `client=desktop` without a valid `dk`, unknown `client` | 400 | `{"error":"bad request"}` | no |
| `AuthError::Unavailable`: database error, JWKS or token endpoint unreachable with no usable key | 503 | `{"error":"service unavailable"}` | error, with cause |
| `CallbackOutcome::Failed` (any refusal or unavailability in the callback) | 302 | `Location: /hub.html?signin=failed` | warn |
| Config: missing or invalid auth variable | exit 1 | `hub-server: missing DATABASE_URL, GOOGLE_CLIENT_ID` | stderr |
| `prepare` failure (connect, checksum, migration) | exit 1 | `hub-server: migration 1 (users): <cause>` | stderr |

`Refusal` is private-ish (`pub` for logs only): `Malformed, BadAlg, UnknownKid, BadSignature,
WrongAudience, WrongIssuer, Expired, MissingClaim, NonceMismatch, TicketMissing, TicketBadMac,
TicketExpired, StateMismatch, IdpRefusedCode, CodeUnknown, CodeExpired, DesktopKeyMismatch`. Every
`/auth/*` and `/admin/*` response carries `Cache-Control: no-store` via `routes::json`.

### Google (auth/google.rs, private)

Verify: `decode_header`; `alg == RS256` and `kid` present, else Malformed or BadAlg;
`jwks.key(kid)`; `jsonwebtoken::decode` with `Validation::new(RS256)`, `set_audience([client_id])`,
`set_issuer(issuers)`, `validate_exp = false` (exp is checked against `AuthClock`, so tests can
expire a token without sleeping), then `exp > now`, `sub` non-empty, `email` present, optional
`nonce` equality. `email_verified` copied as given (absent means false).

JWKS cache: `tokio::sync::Mutex<JwksState { keys: HashMap<String, DecodingKey>, fresh_until,
last_fetch }>`, held across the fetch (single flight; never a std Mutex across `.await`). Fresh and
kid known: use it. Stale: fetch. Fresh and kid unknown: fetch only if `now - last_fetch >= 60 s`,
else UnknownKid. Max-age from `Cache-Control: max-age=N` clamped to 300..=86400 s, absent -> 300 s.
A failed fetch keeps the old keys and counts as `last_fetch` (no hammering); with no keys at all it
is Unavailable.

Exchange: form POST to `token_url` with `code`, `client_id`, `client_secret`, `redirect_uri =
PUBLIC_ORIGIN + "/auth/google/callback"`, `grant_type=authorization_code`, `code_verifier`; a non-2xx
is IdpRefusedCode; the body's `id_token` goes through verify with the ticket's nonce. Authorization
URL: `response_type=code`, `scope=openid email profile`, `state`, `nonce`, `code_challenge =
b64url(SHA-256(verifier))`, `code_challenge_method=S256`, all from 32 random bytes each.

Not done, deliberately: CSRF token (D-010), `sqlx::migrate!` (D-011), compile-time macros, a
session loader on chart, static or `/ws` paths (the socket stays anonymous), `/admin/restart`,
`/admin/metrics`, email-based account joining, any database in local mode.

## Synthesis decision

To be filled by the orchestrator.

## Tradeoffs accepted

- We accept that `Auth` speaks `http` types (`HeaderMap`, `Method`, `Uri`, `SetCookie`) in exchange
  for cookie names, attributes and the gate policy having exactly one home; Google's wire types and
  SQL rows stay private.
- We accept one `auth/mod.rs` of perhaps 500 lines in exchange for each flow reading top to bottom
  as its statement list; `store.rs`, `google.rs`, `crypto.rs` keep the leaves out of it.
- We accept the desktop placeholder session (one extra insert and delete per desktop sign-in) in
  exchange for no new key and an unchanged schema.
- We accept that `/auth/*`, `/api/me*`, `/admin/*` cost one or two indexed single-table reads per
  request (no join allowed) in exchange for no session cache to invalidate on delete or sign-out.
- We accept a manual clock inside `Auth` (separate from `tokio::time`) in exchange for tests that
  move days of time while talking to a real database socket.
- We accept that the rate limiter is extended from `/api/*` to `/auth/*` and `/admin/*` (a
  one-line change in `net::rate_limit`) so One Tap and exchange are not unthrottled DB writers.
- We accept `migrate_down_all` existing only under `test-idp`, so that feature is really "test
  support"; a second feature would split one switch into two that must always move together.

## Alternatives considered

- **Layered modules per spec item 21** (`google.rs`, `session.rs`, `desktop.rs`, `admin.rs` each
  public, with `SessionUser` and `AdminUser` extractors): shallower. Each handler composes
  verify, upsert, session create and cookie building itself, so the sign-in effect is written three
  times (One Tap, callback, exchange) and the slide needs every extractor to remember the response
  cookie. It exposes the pgsqlite rule to every module that holds a pool. Lost on depth.
- **A repository trait (`trait AuthStore`) with in-memory and SQL implementations**: hides SQL
  well, but the in-memory fake would prove nothing about pgsqlite, which is the measured risk; the
  suite must run on both real engines anyway. A trait with one real implementation is a pass-through
  layer. Lost.
- **Per-route extractors instead of the gate middleware**: Origin and admin rules then depend on
  each handler declaring the right extractor; a forgotten one on a new `/admin/` route is an open
  door. The gate makes the safe path the default. Lost.

## Open questions and risks

- Is `CREATE TABLE IF NOT EXISTS` (M1) and `CREATE INDEX` accepted by pgsqlite 0.0.22 through
  `raw_sql`? Not in the measured table; 03-01 task 1 should probe it before anything else.
- Does pgsqlite `--in-memory` share one database across all pool connections, and does it create a
  Unix socket file under `--socket-dir` (default `/tmp`)? If the first fails, test pools use one
  connection; if the second, L-008 needs an exception for the socket file or a pgsqlite flag.
- Does pgsqlite report `rows_affected` correctly for `DELETE` (S13, the one-time guard)? If not,
  the guard falls back to S12 then S13 inside a transaction that pgsqlite serializes anyway; the
  concurrent-exchange test decides.
- Should a refused desktop exchange (wrong key) burn the code? This design burns it (strict one
  time); the cost is that an attacker holding a leaked code can cancel a legitimate sign-in.
- Phase 5 sync will need the pool. Should it get its own deep service built from `Db::clone()` in
  `AppState`, or should `Auth` grow a `db()` accessor? This design prefers the former and keeps `Db`
  public for that reason.
- RSA key generation in `TestIdp` is slow in debug builds; set `[profile.dev.package.rsa]` and
  `num-bigint-dig` to `opt-level = 3`, generate once per test binary (`OnceLock`)?
- Spec item 20 asks for a readiness check in `/api/health`; its body is a Phase 1 parity type.
  Leave health unchanged in Phase 3 (prepare already fails start-up)?

## Next implementation step

03-01 task 1: add `sqlx` and write `db.rs` with M1 to M4 and migration 0001, and run
`db_engines.rs` up, down, up against in-memory pgsqlite before writing any auth code.

## Plan boundaries

**03-01: config, db, migrations.** Crates: `sqlx 0.8` (default-features off; `runtime-tokio`,
`tls-rustls-ring-webpki`, `postgres`, `uuid`, `chrono`), `uuid 1` (`v7`, `serde`), `chrono 0.4`
(`clock`, `serde`, default off), `sha2 0.10`. Work: `AuthMode::Google(Arc<AuthSettings>)`, `Secret`,
`SecretKey`, `HttpsOrigin`, `ConfigError::Missing` listing every name; `AUTH_ENABLED=true` parses
fully but `AppState::build` still refuses it (`AuthNotAvailable` kept until 03-02 removes it);
`cargo` feature `test-idp` declared (enables GOOGLE_* env and the test-support db functions);
`db.rs`; `migrations/0001_users.{up,down}.sql`; `tests/common/db.rs` harness; `tests/db_engines.rs`;
CI job with pgsqlite and a Postgres 17 service. Done when config tests and `db_engines` pass on both
engines and the local suite is unchanged.

**03-02: auth and routes.** Crates: `jsonwebtoken 9`, `hmac 0.12`, `hkdf 0.12`, `base64 0.22`,
`getrandom 0.3`, `subtle 2`; optional under `test-idp`: `rsa 0.9` (with `rand_core 0.6`,
`getrandom` feature). Work: `auth/{mod,store,google,crypto,test_idp}.rs`, `AppState.auth`,
`build_with_auth(cfg, endpoints, deps)`, two route tables, gate middleware, `routes/{auth,me}.rs`,
`/admin/logs*` on the existing handlers, limiter on `/auth/*` and `/admin/*`, `main.rs` prepare and
hourly sweep, redaction of `__Host-vtv_*` values in the log ring, `tests/auth.rs`, `tests/admin.rs`.
`hub-server` gains feature `test-idp` that forwards to hub-core and mounts `TestIdp::router()` at
`/__test-idp/` for 03-03. CI asserts the release build's feature list has no `test-idp`.

**03-03: frontend.** `hub/auth.js` (GIS One Tap with the JavaScript callback POSTing
`{credential}` as JSON, the `sessionStorage` dismissal flag, header button to `/auth/google/start`,
`/auth/me` once on load, sign out, delete account with the mockup 04 confirm), header sign-in
control and avatar menu from mockup 01 (no native `<select>`, tokens only), `log_panel.js` asks
`/auth/me` once and polls `/api/logs` (local), `/admin/logs` (admin) or nothing, the remote-MT5
restart button removed from `hub.html`, `auth.js` added to the static allow-list. Browser check:
`hub-server --features test-idp` with `GOOGLE_*` pointed at `/__test-idp/`, local mode and auth
mode, signed out, member and admin, light and dark.

## Test strategy

Engines: `tests/common/db.rs` has `Engine::{Pgsqlite, Postgres}`. Pgsqlite: spawn `PGSQLITE_BIN`
(default `~/.local/bin/pgsqlite`) with `--in-memory --port 0`-style free port per test, killed on
drop; a missing binary fails the test with its path (never a silent skip). Postgres: `CREATE
DATABASE t_<uuid>` on `TEST_POSTGRES_URL`, dropped on drop. A macro `db_test!(name, |h| ..)` emits
`name_pgsqlite` and `name_postgres`; the Postgres variant is `#[ignore = "needs TEST_POSTGRES_URL"]`
and CI runs `--include-ignored` with the URL set, so local runs show the ignored count honestly.
Auth tests use `AuthDeps::test(&TestIdp, &ManualClock)`: no Google, no IdP socket, no files, no
paused clock. Tests may read the database directly, single-table only (`SELECT id FROM users`).
Auth suites have `required-features = ["test-idp"]`; CI runs `cargo test --workspace` and
`cargo test -p hub-core --features test-idp`.

| Case (item 23 and facts block) | Proof, both engines | Mutation that must turn it red (L-003) |
|---|---|---|
| Wrong `aud` | TestIdp mints with `aud = other`; onetap 401 `sign-in failed` | drop `set_audience` |
| Expired | `exp = now + 60`, clock +61 s; 401 | skip the `exp > now` check |
| Bad signature | sign with a second key under the published kid; 401 | `insecure_disable_signature_validation` |
| Wrong `nonce` | callback where TestIdp's token carries another nonce; 302 `?signin=failed` | skip nonce compare |
| Wrong `iss`, `alg` HS256 or none, missing `sub` | three cases, 401 each | drop issuer list, accept any alg, accept empty sub |
| First sign-in creates one user, second reuses it | two onetaps same `sub`; `users` has 1 row, same `id` both bodies | always take the S2 path |
| Concurrent first sign-ins | two tasks joined; 1 user row, both 200 | drop the second S1 and the retry |
| Profile refresh | second sign-in with a new email updates the row | drop S4 |
| CSRF (now Origin, D-010) | every non-GET gated route: no Origin 403, foreign 403, `PUBLIC_ORIGIN` passes; GET `/auth/me` needs none | let a missing Origin pass |
| JSON only | onetap with `text/plain` 415 | drop the content-type rule |
| Desktop code works once | start `client=desktop&dk=K`, callback 302 to `vtradingview://auth?code=C`, exchange 200 with cookie, again 401 | drop S13 |
| Desktop expiry | clock +61 s, 401 | skip `expires_at` check |
| Desktop wrong key | exchange with `K2`, 401; then with `K`, 401 (burned) | skip the key compare |
| Placeholder unusable | after callback no response carried a session cookie; `sessions` row count 1 with `expires_at` 60 s | give the placeholder 30 d |
| Non-admin 404 on every `/admin/*` | GET logs, POST journal, GET `/admin/x`: signed out, member, admin email with `email_verified=false`, foreign Origin; admin 200 | ignore `email_verified`; check Origin before admin |
| `/api/logs*` gone with auth on, kept locally | auth on 404 both; local mode (no DB) 200 and journal 200 as Phase 1 | mount logs in both tables |
| Session stored hashed | `sessions.id_hash` equals SHA-256 of the cookie, never the raw bytes | store the raw id |
| 24 h slide | +23 h: no Set-Cookie, `last_seen_at` unchanged; +24 h 1 s: Set-Cookie and new row times | slide on every request |
| 30 d expiry | +30 d 1 s with no request: `/auth/me` `user: null` | compare with `<` instead of `<=` |
| Oauth cookie | tampered MAC, `exp` past (+601 s), state mismatch, cookie absent: `?signin=failed`, cookie cleared each time | skip `verify_slice`; skip exp |
| PKCE | TestIdp checks `S256(verifier) == challenge`; a ticket with a swapped verifier fails | send no verifier |
| JWKS cache and kid rule | fetch counter: 2 tokens 1 fetch; rotated kid refetches once; 2 unknown kids within 60 s 1 refetch, after 61 s 2; max-age 10 clamps to 300 | refetch on every unknown kid; no clamp |
| Logout | 204, cookie `Max-Age=0`, session row gone, second logout 204 | drop S9 |
| Delete account | 204; `users`, `identities`, `sessions`, `desktop_auth_codes` rows for the user gone | delete only the session |
| Local mode | `/auth/me` `{"auth": false}`, `/auth/google/start` and `DELETE /api/me` 404; contract suite unchanged | mount auth routes when `auth` is None |
| Config | each missing variable named; `http://` origin refused; 31-byte key refused; GOOGLE_* ignored without feature | drop one required key; read GOOGLE_* always |
| Migrations (`db_engines.rs`) | up, down, up; second up applies nothing; edited checksum refused; failing second statement leaves no table and no M3 row | skip the checksum compare; run outside a tx |
| HTTP IdP path | `TestIdp::router()` on a loopback socket (real time), `AuthDeps::http_with_clock`: onetap 200, Cache-Control parsed | parse max-age as minutes |
| SQL rule | unit test scans `store.rs` and `db.rs` SQL constants for the banned forms | add a join, the test fails |
