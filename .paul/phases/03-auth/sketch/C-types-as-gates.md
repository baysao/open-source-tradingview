---
phase: 03-auth
type: Architecture candidate (architect Phase B)
candidate: C, types as gates
date: 2026-11-24
---

# Candidate C: the type system is the gate

## Problem

Phase 03 adds accounts to a server that today has one router (`app(state)`), no database and an
anonymous `post_guard`. Four things make the shape non-obvious. (1) pgsqlite through sqlx decodes
only single-table statements with no `RETURNING`, join, subquery, cast, function or default (D-014,
measured), so every multi-table effect (sign-in, desktop exchange, delete account) is a sequence of
one-table statements in one transaction, and `sqlx::migrate!` is out (D-011). (2) Two modes must
coexist from one `app()`: local mode has no database and keeps `/api/logs` public with Python's
lenient origin rule (D-001, D-012); hosted mode refuses a missing `Origin` on every non-GET under
`/api/me*`, `/auth/*`, `/admin/*` (D-010) and hides `/admin/*` behind a 404. (3) Authorization
mistakes are silent: a handler that forgets the origin check or the admin check still compiles and
still answers 200 in the happy-path test. (4) Tests must not call Google, must write no files
(L-008), must not open sockets under the paused clock (L-016), and must run on pgsqlite and
Postgres 17. Constraints carried in: `AppState` fields stay cheap clones; `Config::from_lookup` stays
the only env reader; `api_error` bodies and the `Cache-Control: no-store` rule; the rate limiter
(D-007); the Phase 1 `/api/logs` bodies.

## Usage (caller's view)

The reader of `routes/table.rs` sees the whole security policy in one screen. A route is either a
`read` (GET only, any handler) or a `write` (any other method), and a write handler's first
argument must be `Checked<P>`, a proof that `P` (who) and the origin rule (where from) both held:

```rust
// routes/table.rs: the only place a route is registered (a source-scan test enforces it)
pub fn local(r: Routes<AppState>) -> Routes<AppState> {
    r.read("/auth/me", |_: State<AppState>| async { json_ok(&json!({"auth": false})) })
     .read("/api/logs", logs::logs)
     .write(Post, "/api/logs/journal", logs::journal)          // Checked<Anyone>, lenient rule
}
pub fn hosted(r: Routes<Hosted>) -> Routes<Hosted> {
    r.read("/auth/me", auth::me)                                 // Viewer
     .write(Post, "/auth/google/onetap", auth::onetap)           // Checked<Anyone>
     .read("/auth/google/start", auth::start)                    // sets the MAC'd ticket
     .read("/auth/google/callback", auth::callback)              // OAuthReturn proves state
     .write(Post, "/auth/desktop/exchange", auth::exchange)      // Checked<Anyone>
     .write(Post, "/auth/logout", auth::logout)                  // Checked<Anyone> + Viewer
     .write(Delete, "/api/me", me::delete)                       // Checked<SignedIn>
     .read("/admin/logs", admin::logs)                           // Admin (404)
     .write(Post, "/admin/logs/journal", admin::journal)         // Checked<Admin>
}
```

Three real handlers, as they will be written in 03-02:

```rust
// routes/me.rs: DELETE /api/me. Signed out never reaches the body; neither does a foreign Origin.
pub async fn delete(c: Checked<SignedIn>, _: ()) -> Result<Response, Reject> {
    db::users::delete(c.st.auth.db(), c.who.user().id).await?;   // cascade: identities, sessions, codes
    Ok(cookies::cleared(StatusCode::NO_CONTENT))
}

// routes/auth.rs: One Tap. Only `Idp::verify` can make a `VerifiedIdentity`, and only a
// `VerifiedIdentity` can create a user or a session.
pub async fn onetap(c: Checked<Anyone>, JsonBody(b): JsonBody<OneTap>) -> Result<Response, Reject> {
    let id = c.st.auth.idp().verify(&b.credential, None).await?;
    let s = c.st.auth.sign_in(&id, c.user_agent()).await?;       // one transaction
    Ok(s.into_response())                                         // {"user", "admin"} + Set-Cookie
}

// routes/admin.rs: the Phase 1 handler, unchanged, behind the 404 proof.
pub async fn logs(_: Admin, st: State<AppState>, q: Query<HashMap<String, String>>) -> Response {
    crate::routes::logs::logs(st, q).await
}
```

`hub-server` start-up, the only async construction:

```rust
let state = AppState::start(cfg).await?;   // local: no I/O; hosted: connect, migrate, build Auth
```

## Shape

### Data structures

```rust
// config.rs (03-01)
pub enum AuthMode { Off, Google(Box<AuthConfig>) }           // replaces AuthMode::Off only
pub struct AuthConfig {
    pub database_url: Secret<String>,
    pub client_id: String,
    pub client_secret: Secret<String>,
    pub secret_key: SecretKey,          // >= 32 bytes after base64url decode
    pub public_origin: HttpsOrigin,     // "https://..." without trailing slash; test-idp also allows http://127.0.0.1:<port>
    pub admin_emails: AdminEmails,      // lowercased, trimmed, empty by default
    #[cfg(feature = "test-idp")] pub idp_urls: Option<IdpUrls>,   // GOOGLE_ISSUER, _JWKS_URL, _TOKEN_URL, _AUTH_URL
}
pub struct Secret<T>(T);                // Debug prints "[redacted]"; no Display
pub enum ConfigError { Invalid { key: &'static str, value: String }, Missing(Vec<&'static str>) }
// AuthNotAvailable is deleted. Missing lists every absent variable at once, in spec order.

// db/mod.rs (03-01)
#[derive(Clone)] pub struct Db(PgPool);                       // only `db::open` makes one: proof of a current schema
pub struct Migration { pub version: i64, pub name: &'static str, pub up: &'static str, pub down: &'static str }
pub const MIGRATIONS: &[Migration] = &[Migration { version: 1, name: "users",
    up: include_str!("../../../../migrations/0001_users.up.sql"),
    down: include_str!("../../../../migrations/0001_users.down.sql") }];

// auth/mod.rs (03-02)
#[derive(Clone)] pub struct Auth(Arc<AuthInner>);           // cfg, db, idp, keys (HKDF), now
#[derive(Clone)] pub struct Hosted { pub app: AppState, pub auth: Auth }   // impl FromRef<Hosted> for AppState
pub struct SessionId([u8; 32]);       // the secret; leaves the process only inside Set-Cookie; no Debug
pub struct SessionHash([u8; 32]);     // SHA-256 of SessionId; the only form the store accepts
pub struct VerifiedIdentity { subject: String, email: String, email_verified: bool, name: Option<String>, picture: Option<String> }
pub struct User { pub id: Uuid, pub email: String, pub email_verified: bool, pub name: Option<String>, pub picture: Option<String> }
pub enum Now { System, #[cfg(feature = "test-idp")] Manual(Arc<AtomicI64>) }   // ms since epoch

// auth/gates.rs (03-02): every field private; constructors are the checks
pub enum Viewer { Anonymous, Signed(SignedIn) }
pub struct SignedIn { user: User, session: SessionHash, admin: bool }
pub struct Admin(SignedIn);
pub struct Anyone;
pub struct Checked<P, S = Hosted> { pub who: P, pub st: S }    // local writes: Checked<Anyone, AppState>
pub struct OAuthReturn { ticket: OAuthTicket, code: String }
pub struct JsonBody<T>(pub T);  pub struct JsonBytes(pub Bytes);

// auth/ticket.rs: the __Host-vtv_oauth payload (JSON, base64url, then "." and base64url HMAC)
pub struct OAuthTicket { state: String, nonce: String, verifier: String, client: Client, dk_hash: Option<[u8; 32]>, exp: i64 }
pub enum Client { Web, Desktop }
```

Load-bearing decisions:

1. **The mode is a type, not a flag (per encode-lessons-in-structure).** Local routes are
   `Routes<AppState>`; hosted routes are `Routes<Hosted>`, a state in which `Auth` is not optional.
   `SignedIn`, `Admin` and `OAuthReturn` implement `FromRequestParts<Hosted>` only, so a local route
   that names them does not compile. The origin rule is chosen by the state type through a sealed
   trait: `impl OriginRule for AppState` is Python's lenient rule (missing allowed, localhost and
   127.0.0.1 accepted), `impl OriginRule for Hosted` is D-010 (exact `PUBLIC_ORIGIN`, missing refused).
   No route carries a mode `if`. A route that should not exist in a mode is absent, so it falls to
   the fallback 404, which now covers `/api/`, `/auth/` and `/admin/` with `{"error":"not found"}`.
2. **One builder, two shapes.** `app(state)` is still the only router function:
   `Routes::new(shared)` then `match state.auth { None => local(..), Some(a) => hosted(..)
   .with_state(Hosted{..}) }`, merged, then the session layer (hosted only), rate limit, headers.
   `Routes` wraps a private `Router` and a `Vec<RouteInfo { method, path, gate }>` manifest exposed as
   `hub_core::route_manifest(&state)`; tests enumerate it (see Test strategy).
3. **`write` takes only `Fn(Checked<P>, A) -> Fut`.** The bound is `P: Proof<S>`,
   `A: FromRequest<S, M>`, `S: OriginRule`, which is exactly axum's two-argument `Handler` impl, so
   no tuple tricks are needed. `Checked<P>` resolves `P` first, then the origin: on `/admin/*` the 404
   wins over the 403 (an outsider learns nothing), on `/api/me` a signed-out caller gets 401. The body
   extractor runs last (415, 413, 400). `read` registers `get(h)` and nothing else.
4. **`Viewer` is resolved at most once per request, lazily.** The hosted session layer inserts a
   `ViewerCell(Arc<OnceCell<Viewer>>)` plus a `CookieAction` slot into the request extensions and
   runs the route. The first extractor that needs the viewer fills the cell: no cookie, no query;
   malformed or unknown cookie, `Anonymous` and `CookieAction::Clear`; expired, delete the row,
   `Anonymous`, `Clear`; alive and `now - last_seen_at >= 24 h`, slide (`expires_at = now + 30 d`,
   `last_seen_at = now`) and `CookieAction::Refresh(id)` so the cookie's `Max-Age` moves with the row.
   After the handler the layer appends the `Set-Cookie`, unless the handler already set or cleared
   the session cookie (sign-in, logout, delete win). Static files never touch the database.
   `SignedIn` and `Admin` read the same cell; a missing cell is a wiring bug and answers 500 in debug
   tests. `/ws` does not take a `Viewer` in Phase 3 (charts must not depend on database health).
5. **Data access is per-table modules of one-statement async functions** taking
   `impl PgExecutor<'_>` (`&PgPool` or `&mut *tx`). Each module exposes its SQL as `const`s listed in
   `STATEMENTS: &[&str]`, so one test can audit every statement the server can issue (D-014 as a
   test, not a comment). Sequences that touch several tables live in `auth` (`sign_in`,
   `issue_desktop_code`, `exchange_desktop_code`) and open one transaction each. Call chain from a
   handler: `routes/auth.rs` to `auth/mod.rs` to `db/<table>.rs`, three files.
6. **Secrets cannot be stored by mistake.** Store functions take `&SessionHash`, never `&SessionId`;
   `SessionId::hash()` is the only way to get a hash; desktop codes and keys follow the same pattern
   (`CodeHash`, `KeyHash`). `VerifiedIdentity` has no public constructor, so the only path to a user
   row is through `Idp::verify` (or the test signer, which still goes through `verify`).
7. **A `Db` value means "migrated".** `db::open(url).await` connects (pool of 5, acquire timeout
   5 s) and runs the migration runner; `AppState::start` is the only caller. `AppState::build` stays
   sync and local-only; given an auth-on config it returns `ConfigError::Invalid{key:"AUTH_ENABLED"}`
   with the hint to use `start`. Tests attach auth with `AppState::with_auth(self, Auth)` under the
   `test-idp` feature.

### Signatures

```rust
// db/mod.rs
pub async fn open(url: &Secret<String>) -> Result<Db, DbError> { todo!() }   // connect, then migrate
pub async fn migrate(pool: &PgPool) -> Result<Vec<i64>, DbError> { todo!() } // returns versions applied now
/// Tests only (db_engines.rs): applies down files from the newest to `target` exclusive.
pub async fn migrate_down_to(pool: &PgPool, target: i64) -> Result<(), DbError> { todo!() }
pub fn is_unique_violation(e: &sqlx::Error) -> bool { todo!() }   // 23505, or 42000 + "UNIQUE constraint failed"
#[doc(hidden)] pub fn all_statements() -> Vec<&'static str> { todo!() }
pub enum DbError { Connect(sqlx::Error), Query(sqlx::Error), ChecksumChanged { version: i64 }, UnknownApplied { version: i64 } }

// db/users.rs, identities.rs, sessions.rs, desktop_codes.rs (one statement each)
pub async fn users::insert(e: impl PgExecutor<'_>, u: &UserRow) -> Result<(), sqlx::Error> { todo!() }
pub async fn users::update_login(e: impl PgExecutor<'_>, id: Uuid, v: &Profile, at: DateTime<Utc>) -> Result<(), sqlx::Error> { todo!() }
pub async fn users::get(e: impl PgExecutor<'_>, id: Uuid) -> Result<Option<User>, sqlx::Error> { todo!() }
pub async fn users::delete(e: impl PgExecutor<'_>, id: Uuid) -> Result<(), sqlx::Error> { todo!() }
pub async fn identities::insert_if_absent(e: impl PgExecutor<'_>, sub: &str, user: Uuid, at: DateTime<Utc>) -> Result<(), sqlx::Error> { todo!() }
pub async fn identities::user_of(e: impl PgExecutor<'_>, sub: &str) -> Result<Option<Uuid>, sqlx::Error> { todo!() }
pub async fn sessions::insert(e: impl PgExecutor<'_>, s: &SessionRow) -> Result<(), sqlx::Error> { todo!() }
pub async fn sessions::get(e: impl PgExecutor<'_>, h: &SessionHash) -> Result<Option<SessionRow>, sqlx::Error> { todo!() }
pub async fn sessions::slide(e: impl PgExecutor<'_>, h: &SessionHash, expires: DateTime<Utc>, seen: DateTime<Utc>) -> Result<(), sqlx::Error> { todo!() }
pub async fn sessions::delete(e: impl PgExecutor<'_>, h: &SessionHash) -> Result<(), sqlx::Error> { todo!() }
pub async fn sessions::delete_expired(e: impl PgExecutor<'_>, now: DateTime<Utc>) -> Result<u64, sqlx::Error> { todo!() }
pub async fn desktop_codes::insert(e: impl PgExecutor<'_>, c: &CodeRow) -> Result<(), sqlx::Error> { todo!() }
pub async fn desktop_codes::get(e: impl PgExecutor<'_>, h: &CodeHash) -> Result<Option<CodeRow>, sqlx::Error> { todo!() }
pub async fn desktop_codes::delete(e: impl PgExecutor<'_>, h: &CodeHash) -> Result<bool, sqlx::Error> { todo!() }

// auth/mod.rs
impl Auth {
    pub fn new(cfg: AuthConfig, db: Db, idp: Idp, now: Now) -> Auth { todo!() }   // derives the HKDF keys
    pub fn db(&self) -> &PgPool { todo!() }
    pub fn idp(&self) -> &Idp { todo!() }
    /// One transaction: identity lookup, user insert or update, identity insert, session insert.
    pub async fn sign_in(&self, id: &VerifiedIdentity, ua: Option<&str>) -> Result<NewSession, Reject> { todo!() }
    /// Callback, desktop client: sign_in with a pending session (expires with the code, 60 s, id discarded) plus the code row.
    pub async fn issue_desktop_code(&self, id: &VerifiedIdentity, dk: &KeyHash, ua: Option<&str>) -> Result<DesktopCode, Reject> { todo!() }
    /// One transaction: read and delete the code (consumed on every attempt), check expiry and key,
    /// read the pending session, insert a fresh session, delete the pending one.
    pub async fn exchange_desktop_code(&self, code: &str, dk: &str, ua: Option<&str>) -> Result<NewSession, Reject> { todo!() }
    pub(crate) async fn resolve(&self, cookie: Option<&str>) -> Result<(Viewer, CookieAction), Reject> { todo!() }
}
pub fn admits(emails: &AdminEmails, u: &User) -> bool { todo!() }   // pure: email_verified && lowercase email listed
pub struct NewSession { id: SessionId, user: User, admin: bool }    // IntoResponse: 200 {"user","admin"} + Set-Cookie

// auth/google.rs
impl Idp {
    pub fn google(http: reqwest::Client) -> Idp { todo!() }                // fixed URLs, issuers accounts.google.com and https://accounts.google.com
    #[cfg(feature = "test-idp")] pub fn from_urls(http: reqwest::Client, u: &IdpUrls) -> Idp { todo!() }
    #[cfg(feature = "test-idp")] pub fn in_process(t: TestIdp) -> Idp { todo!() }   // no sockets
    pub fn authorize_url(&self, client_id: &str, redirect: &str, t: &OAuthTicket) -> String { todo!() }
    pub async fn exchange_code(&self, cfg: &AuthConfig, code: &str, verifier: &str) -> Result<String, Reject> { todo!() } // returns id_token
    /// RS256 against the cached JWKS, aud, iss, exp (60 s leeway), and nonce when given.
    pub async fn verify(&self, token: &str, nonce: Option<&str>) -> Result<VerifiedIdentity, Reject> { todo!() }
}
/// Pure cache policy, unit-tested without I/O.
pub fn jwks_plan(now: i64, fetched_at: i64, max_age_s: i64, kid_known: bool, last_kid_refetch: Option<i64>) -> JwksPlan { todo!() }
pub enum JwksPlan { Use, Refetch, Refuse }   // stale: Refetch; unknown kid: Refetch at most once per 60 s, else Refuse
pub fn clamp_max_age(cache_control: Option<&str>) -> i64 { todo!() }  // 300..=86400, missing means 300

// auth/ticket.rs
impl OAuthTicket {
    pub fn new(client: Client, dk: Option<&str>, now: i64) -> OAuthTicket { todo!() }   // 32 random bytes each, exp now+600
    pub fn seal(&self, key: &MacKey) -> String { todo!() }
    pub fn open(raw: &str, key: &MacKey, now: i64) -> Result<OAuthTicket, TicketError> { todo!() }  // MAC (constant time), then exp
    pub fn challenge(&self) -> String { todo!() }   // base64url(SHA-256(verifier)), method S256
}

// auth/gates.rs
pub trait OriginRule: sealed::Sealed { fn origin_ok(&self, headers: &HeaderMap) -> bool; }
pub trait Proof<S>: Sized + Send + sealed::Sealed { fn prove(parts: &mut Parts, st: &S) -> impl Future<Output = Result<Self, Reject>> + Send; }
impl<S: OriginRule + Clone + Send + Sync, P: Proof<S>> FromRequestParts<S> for Checked<P, S> { type Rejection = Reject; /* P, then origin */ }
impl FromRequestParts<Hosted> for Viewer { type Rejection = Reject; }
impl FromRequestParts<Hosted> for SignedIn { type Rejection = Reject; }   // Anonymous: NotSignedIn
impl FromRequestParts<Hosted> for Admin { type Rejection = Reject; }      // anything else, DB errors included: NotFound
impl FromRequestParts<Hosted> for OAuthReturn { type Rejection = Reject; } // cookie MAC, exp, state equality
pub async fn session_layer(State(h): State<Hosted>, req: Request, next: Next) -> Response { todo!() }

// routes/table.rs
impl<S: OriginRule + Clone + Send + Sync + 'static> Routes<S> {
    pub fn read<H: Handler<T, S>, T: 'static>(self, path: &'static str, h: H) -> Self { todo!() }
    pub fn write<P, A, M, F, Fut, R>(self, m: Write, path: &'static str, f: F) -> Self
    where P: Proof<S> + 'static, A: FromRequest<S, M> + Send + 'static, M: 'static,
          F: Fn(Checked<P, S>, A) -> Fut + Clone + Send + Sync + 'static,
          Fut: Future<Output = R> + Send + 'static, R: IntoResponse + 'static { todo!() }
}
pub enum Write { Post, Put, Delete }
```

### SQL: migration 0001 and every statement

`migrations/0001_users.up.sql` (plain DDL, no defaults, no functions; ids are UUID v7 and times are
written by Rust, D-011):

```sql
CREATE TABLE users (id uuid PRIMARY KEY, email text NOT NULL, email_verified boolean NOT NULL,
  name text NULL, picture text NULL, created_at timestamptz NOT NULL, last_login_at timestamptz NOT NULL);
CREATE TABLE identities (provider text NOT NULL, subject text NOT NULL,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE, created_at timestamptz NOT NULL,
  PRIMARY KEY (provider, subject));
CREATE INDEX identities_user_id ON identities (user_id);
CREATE TABLE sessions (id_hash bytea PRIMARY KEY, user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL, expires_at timestamptz NOT NULL, last_seen_at timestamptz NOT NULL,
  user_agent text NULL);
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE TABLE desktop_auth_codes (code_hash bytea PRIMARY KEY, desktop_key_hash bytea NOT NULL,
  session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE, expires_at timestamptz NOT NULL);
```

`0001_users.down.sql`: `DROP TABLE desktop_auth_codes; DROP TABLE sessions; DROP TABLE identities;
DROP TABLE users;` (indexes go with their tables).

| Id | Module | Statement |
|---|---|---|
| M1 | migrations | `CREATE TABLE IF NOT EXISTS schema_migrations (version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL, applied_at timestamptz NOT NULL)` |
| M2 | migrations | `SELECT version, name, checksum FROM schema_migrations` (sorted in Rust) |
| M3 | migrations | the file body through `sqlx::raw_sql` inside the file's transaction |
| M4 | migrations | `INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)` |
| M5 | migrations | `DELETE FROM schema_migrations WHERE version = $1` (down, tests) |
| U1 | users | `INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)` |
| U2 | users | `UPDATE users SET email = $2, email_verified = $3, name = $4, picture = $5, last_login_at = $6 WHERE id = $1` |
| U3 | users | `SELECT id, email, email_verified, name, picture FROM users WHERE id = $1` |
| U4 | users | `DELETE FROM users WHERE id = $1` |
| I1 | identities | `INSERT INTO identities (provider, subject, user_id, created_at) VALUES ('google', $1, $2, $3) ON CONFLICT (provider, subject) DO NOTHING` |
| I2 | identities | `SELECT user_id FROM identities WHERE provider = 'google' AND subject = $1` |
| S1 | sessions | `INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)` |
| S2 | sessions | `SELECT user_id, expires_at, last_seen_at FROM sessions WHERE id_hash = $1` |
| S3 | sessions | `UPDATE sessions SET expires_at = $2, last_seen_at = $3 WHERE id_hash = $1` |
| S4 | sessions | `DELETE FROM sessions WHERE id_hash = $1` |
| S5 | sessions | `DELETE FROM sessions WHERE expires_at < $1` (hourly task; cascades pending desktop codes) |
| D1 | desktop_codes | `INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)` |
| D2 | desktop_codes | `SELECT desktop_key_hash, session_id_hash, expires_at FROM desktop_auth_codes WHERE code_hash = $1` |
| D3 | desktop_codes | `DELETE FROM desktop_auth_codes WHERE code_hash = $1` |

Sequences (each one transaction): **sign-in** I2; on a miss U1 (new id), I1, I2 again, and if the
row names another user (a concurrent first sign-in won) U4 on ours and U2 on the winner; on a hit
U2; then S1. **Viewer** S2, then U3, then S3 or S4 as the slide rule says (no transaction; each is
idempotent). **Desktop issue** sign-in with a pending S1 (expires in 60 s), then D1.
**Desktop exchange** D2, D3 (committed even when the checks fail: a code is spent by any attempt),
S2 on the pending hash, S1 fresh, S4 pending. **Logout** S4. **Delete account** U4. Tests alone
issue `SELECT count(*) FROM <table>` (measured working; never in `STATEMENTS`).

The runner: M1, M2; for every applied row the checksum (SHA-256 of `up`) must equal the embedded
one, else `ChecksumChanged` (start-up fails with the version); an applied version not embedded is
`UnknownApplied`. Each pending file: `begin`, M3, M4, `commit`. Two servers racing collide on M4's
primary key, one rolls back and fails start-up with a clear message (`is_unique_violation`); no
advisory lock exists on pgsqlite.

### Error mapping

`Reject` is the one error type of `auth` and the gates; `impl IntoResponse for Reject` is the whole
map. Reasons are logged at `warn` (target `auth`, never a token, code or cookie value), never returned.

| `Reject` | Raised by | Status and body |
|---|---|---|
| `ForbiddenOrigin` | `Checked<P>` origin rule | 403 `{"error":"forbidden origin"}` |
| `JsonOnly` / `TooLarge` / `BadBody` | `JsonBody`, `JsonBytes` | 415 `JSON only` / 413 `too large` / 400 `bad request` |
| `NotSignedIn` | `SignedIn` | 401 `{"error":"not signed in"}` |
| `NotFound` | `Admin` (any cause, DB errors included) | 404 `{"error":"not found"}` |
| `SignInFailed(reason)` | `verify`, desktop exchange | 401 `{"error":"sign-in failed"}` |
| `IdpUnavailable` | JWKS or token endpoint down | 503 `{"error":"sign-in unavailable"}` |
| `Db(sqlx::Error)` | any store call | 503 `{"error":"database unavailable"}` |
| any of the above on `start` or `callback` | `OAuthReturn`, handlers | 302 `/hub.html?signin=failed`, ticket cookie cleared |

Start-up: `ConfigError::Missing(keys)` prints `AUTH_ENABLED=true requires DATABASE_URL, VTV_SECRET_KEY`;
`DbError` prints the version or the connect error with the URL's password removed; exit code 1.

### What it deliberately does not do

No CSRF token (D-010). No `sqlx::migrate!`, no `query!` macros (builds need no database). No
`RETURNING`, joins, subqueries, casts, functions or defaults (D-014). No eager session lookup for
static files. No `Viewer` on `/ws` in Phase 3. No `/admin/restart`, `/admin/metrics`, `sync.js`.
Interface depth: the public surface is `Routes::{read, write}`, five extractor types, `Auth` with
four operations and `Idp::verify`; it hides the cookie formats, HKDF, PKCE, JWKS caching, the slide
and the multi-statement sequences.

## Synthesis decision

To be filled by the orchestrator.

## Tradeoffs accepted

- We accept a custom `Routes` builder in place of plain `Router::route` in exchange for a write route
  that cannot exist without an origin proof; the cost is one small file and one source-scan test.
- We accept a 404 instead of 403 for a cross-origin POST to `/admin/*` by a non-admin (proof before
  origin) in exchange for admin routes that reveal nothing to outsiders.
- We accept that a cross-site POST carrying no session cookie still costs a session lookup only when
  a cookie is present (SameSite=Lax withholds it cross-site, so in practice none).
- We accept two reads per signed-in request (S2 then U3, no join) in exchange for pgsqlite parity.
- We accept that a desktop code is burned by a wrong-key attempt, in exchange for no brute force on
  `desktop_key`; the app retries sign-in from scratch.
- We accept that the desktop callback's first session is a 60 s placeholder rotated at exchange,
  because the store keeps only hashes and so cannot hand the original id to the app.
- We accept `AppState::start` (async) beside the sync `build`, so that a `Db` always means migrated.

## Alternatives considered

- **One router with runtime guards** (`require_admin(&state, &headers)?` at the top of each handler,
  `Option<Auth>` checked in place). Smaller diff, but every guard is a line a reviewer must notice;
  forgetting one compiles and passes the happy-path test. It hides nothing from handlers.
- **Middleware by path prefix** (`/admin/*` layer, `/auth/*` origin layer). One place, but it keys on
  strings, so a route mounted under the wrong prefix loses its policy silently, and handlers still
  cannot prove to the compiler they are signed in (they would reach into extensions and `unwrap`).
- **A repository trait (`trait Store`) with an in-memory fake for most tests.** Faster tests, but the
  fake cannot reproduce pgsqlite's decoding limits, which are the measured risk of this phase; the
  suite must run real SQL on both engines anyway, so the trait is a pass-through layer.

## Open questions and risks

- Risk: axum's `Handler` blanket impl must accept `Fn(Checked<P>, A)` through the generic `write`
  bound with the `M` marker; if inference fails, the fallback is `write_parts` and `write_body`
  variants (two arities). Proven by a compile test in the first 03-02 task.
- Risk: `CREATE TABLE IF NOT EXISTS` (M1) and `CREATE INDEX` on pgsqlite 0.0.22 were not in the probe;
  03-01 task 1 probes both. Fallback for M1: treat SQLSTATE `42P01` from M2 as "no table".
- Risk: `pgsqlite --in-memory` must share one database across pool connections and must create no
  socket file; if either fails, the pgsqlite pool is capped at 1 connection, or the test spawns it
  with `--socket-dir` inside the scratch temp (a file, needs an L-008 exception).
- Should the desktop browser also stay signed in after the callback (today: no, only the app)?
- The desktop window posts to `/auth/desktop/exchange`: is its `Origin` `PUBLIC_ORIGIN` (the app loads
  the hosted pages) or `tauri://localhost`? D-010 refuses the latter; Phase 6 must confirm.
- Should `/api/health` report database readiness (item 20) or stay Python-parity until Phase 4?
- Should the rate limiter cover `/auth/*` and `/admin/*` as well as `/api/*`? This candidate says yes.

## Plan boundaries

- **03-01 config, db, migrations.** `config.rs` (`AuthMode::Google`, `AuthConfig`, `Secret`,
  `ConfigError::Missing`, `test-idp` URL overrides), `db/` (runner, `Db`, the four table modules
  with `STATEMENTS`), `migrations/0001_users.{up,down}.sql`, `AppState::start` and the `auth` field
  (holding `Db` only, routes untouched), `hub-server` migrating at start-up, `tests/common/db.rs`
  (engines and the `engines!` macro), `tests/db_engines.rs`, `tests/sql_rule.rs`, `.env.example`.
  Accept: local mode byte-identical; hosted config boots against pgsqlite and Postgres 17.
- **03-02 auth and routes.** `auth/{mod,google,ticket,gates,test_idp}.rs`, `routes/{table,auth,me,
  admin}.rs`, the fallback prefixes, the session layer, the hourly S5 sweep, the `test-idp` routes
  (`/__test-idp/authorize|token|jwks`, feature only) for the browser check, `tests/auth.rs`,
  `tests/admin.rs`, `tests/route_gate.rs`.
- **03-03 frontend.** `hub/auth.js` (GIS One Tap with the JS callback, header button, `/auth/me` once,
  sign out, delete account), header sign-in control and avatar menu (mockups 01, 04), `log_panel.js`
  gating (`auth:false` polls `/api/logs`, admin polls `/admin/logs`, others nothing), and the browser
  check against `hub-server --features test-idp` with `PUBLIC_ORIGIN=http://127.0.0.1:<port>`.

Crates (workspace, versions as semver lines, pinned by `Cargo.lock`): `sqlx 0.8`
(`default-features = false`, `runtime-tokio`, `postgres`, `uuid`, `chrono`; no `macros`, no TLS on
loopback), `uuid 1` (`v7`), `chrono 0.4` (`clock`, `serde`), `jsonwebtoken 9`, `sha2 0.10`,
`hmac 0.12`, `hkdf 0.12`, `base64 0.22`, `getrandom 0.2`, `subtle 2`; optional `rsa 0.9` behind
`test-idp`. URLs use `reqwest::Url`; form posts use `reqwest` 0.12's `form`. No cookie crate.

## Test strategy

Engines: `tests/common/db.rs` gives `TestDb::{pgsqlite, postgres}`. pgsqlite: spawn `PGSQLITE_BIN`
(default `~/.local/bin/pgsqlite`) with `--in-memory -p <free port>`, killed on drop. Postgres: from
`VTV_TEST_POSTGRES_URL`, create `vtv_t_<uuidv7>` per test, drop on teardown. `engines!(fn name(db))`
expands to `name_pgsqlite` and `name_postgres`. An absent engine prints `SKIPPED` unless
`VTV_TEST_REQUIRE=pgsqlite,postgres` (CI sets it), which turns the skip into a failure. Time is
`Now::Manual` (no paused tokio clock, L-016); the IdP is `Idp::in_process(TestIdp)` (a 2048-bit key
generated once per test binary, its JWKS served from memory with a fetch counter): no Google, no
files. `cargo test --workspace` runs local suites; `cargo test -p hub-core --features test-idp` adds
the auth suites (`required-features`). Every check has a named mutation run once by hand (L-003).

| Case (item 23 and the facts block) | Proof | Mutation that must fail it |
|---|---|---|
| Forged token: wrong `aud`, expired, bad signature, wrong `iss`, wrong `nonce`, unknown `kid` | TestIdp signs each variant, POST onetap or callback: 401 `sign-in failed` or the failed redirect, users count 0 | remove each check in `verify` in turn |
| First sign-in creates one user, second reuses it | two onetaps for one `sub`: users count 1, same id, `last_login_at` moved, sessions count 2 | skip the first I2 (always U1) |
| Concurrent first sign-in | two `sign_in` futures joined: users count 1 (Postgres; pgsqlite may serialize) | drop the second I2 check |
| Origin rule on every write | enumerate `route_manifest` writes: hosted with missing and with foreign `Origin` 403 (after a valid proof), local with foreign 403 and missing allowed | `Checked` skips the origin call; a write added on the shared routes |
| Only `table.rs` registers routes | `route_gate.rs` scans `src/**` for `.route(`, `routing::post`, `routing::delete` outside `routes/table.rs` | add a raw `.route` in `lib.rs` |
| Desktop code: reuse, expiry, wrong key | exchange twice (second 401), at +61 s (401), wrong key (401, then right key also 401) | skip D3; skip expiry; skip the key compare |
| Desktop code success | callback redirects to `vtradingview://auth?code=`, exchange 200 with cookie, pending session gone | skip S4 on the pending row |
| Every `/admin/*` is 404 for others | enumerate manifest `/admin/` routes: signed out, non-admin, admin email with `email_verified=false` all 404 `not found`; admin 200 | `admits` ignores `email_verified`; `Admin` answers 403 |
| Local mode needs no database | local state from `build_with`: `/auth/me` `{"auth":false}`, `/auth/google/start`, `/admin/logs`, `DELETE /api/me` all 404, existing logs tests unchanged | register a hosted route in `local` |
| Hosted hides `/api/logs*` | hosted: `GET /api/logs`, `POST /api/logs/journal` 404 | mount `logs::logs` in shared routes |
| Session slide once per 24 h | `/auth/me` at +23 h: no Set-Cookie, `last_seen_at` unchanged; at +25 h: one Set-Cookie, row moved; 31 d after the slide: anonymous, cookie cleared, row deleted | slide on every request; skip the expiry check |
| Only hashes stored | sessions row `id_hash` equals SHA-256 of the cookie value and never the value | store the raw id |
| OAuth ticket | start: 302 with `state`, `nonce`, `code_challenge`, `S256`; callback with flipped MAC byte, wrong `state`, ticket at +601 s: failed redirect; good: 302 `/hub.html`, session set, ticket cleared | skip the MAC compare; skip exp |
| JWKS cache | unit tests on `jwks_plan` and `clamp_max_age`; integration: two unknown-kid tokens 10 s apart cause one fetch, 61 s apart two | remove the 60 s limit |
| Logout and delete | logout 204 clears cookie, S2 miss; signed-out logout 204; delete 204, every table count 0 for that user | delete forgets the cookie clear; U4 replaced by S4 |
| Config | auth on with each required variable removed names it; `http://` origin and a 31-byte key refused; without `test-idp` the `GOOGLE_*_URL` vars are ignored (lib unit test, default features) | `Missing` reports only the first key |
| Migrations up and down | `db_engines.rs`: up, schema_migrations has 1 row, down to 0 (all tables gone), up again; changed checksum refuses start | drop a `DROP TABLE` from the down file |
| Single-table SQL rule | `sql_rule.rs` over `all_statements()` and the up files: exactly one known table name each, no `RETURNING`, `JOIN`, `(SELECT`, `::`, `CAST`, `DEFAULT`, `now(` | add `RETURNING id` to U1 |

## Next implementation step

Write `tests/sql_rule.rs` and the `engines!` harness first, then `migrations/0001_users.up.sql` and
the runner until `db_engines.rs` is green on both engines.
