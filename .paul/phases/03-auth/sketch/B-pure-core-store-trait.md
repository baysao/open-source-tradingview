---
phase: 03-auth
type: Architecture candidate (architect Phase B)
candidate: B, pure core, effects at the edge
date: 2026-11-23
---

# Candidate B: pure sign-in rules, a `Store` trait over domain records, an injected IdP

## Problem

Phase 03 adds Google sign-in, a database and an admin gate to a server whose only mode today is
local (D-001: no database, `AUTH_ENABLED=true` refused, `/api/logs` public). Three things make the
shape non-obvious. (1) pgsqlite through sqlx decodes only single-table statements (D-014): every
multi-table effect (first sign-in writes `users` and `identities`, desktop exchange touches codes and
sessions) is a sequence of plain statements in one transaction, and `RETURNING`, joins, subqueries,
casts, functions and column defaults are out. (2) Most rules are time rules (token `exp`, the 600 s
sign-in cookie, the 60 s desktop code, the 24 h slide, the 30-day session, the JWKS TTL clamp and the
once-per-minute `kid` refetch), and tests must hit every edge without Google, files or sockets under
the paused clock (L-008, L-016). (3) Local mode must remain byte-identical and must not need a
database, while auth mode must fail closed (logs never public by accident). Constraints carried in:
`AppState` fields are cheap clones and `AppState::build` does no I/O; `app(state)` is the only
router; `Config::from_lookup` is the only env reader; `post_guard` keeps its Phase 1 meaning on
`/api/logs/journal`; no CSRF token (D-010); own migration runner (D-011); no compile-time sqlx macros.

## Usage (caller's view)

```rust
// hub-server/src/main.rs: the only place that opens sockets for auth
let auth = match &cfg.auth {
    AuthMode::Off => None,
    AuthMode::Google(ac) => {
        let pool = db::connect(&ac.database_url).await?;           // PgPool, no I/O beyond connect
        db::migrate(&pool, Utc::now()).await?;                       // embedded 0001.., one tx per file
        Some(Auth::new(&cfg, Arc::new(SqlStore::new(pool)), Arc::new(GoogleIdp::new(&ac.idp)), WallClock::System)?)
    }
};
let state = AppState::build(cfg, auth)?;      // refuses Google mode without Auth and Off mode with it

// routes/auth.rs: every handler is a parse, one deep call, a render
pub async fn onetap(State(st): State<AppState>, ua: UserAgent, Json(b): Json<Credential>) -> Response {
    let auth = st.auth.as_ref().expect("route mounted only with auth");
    match auth.onetap(&b.credential, ua.0.as_deref()).await {
        Ok(s) => signed_in(s),                 // 200 {"user", "admin"} + Set-Cookie __Host-vtv_session
        Err(e) => e.into_response(),           // 401 {"error":"sign-in failed"} | 503, reason logged
    }
}
pub async fn admin_logs(_: AdminUser, st: State<AppState>, q: Query<HashMap<String, String>>) -> Response {
    routes::logs::logs(st, q).await         // AdminUser rejects with 404 {"error":"not found"}
}

// tests/auth.rs: no Google, no DB, no files, paused clock is allowed (no sockets anywhere)
let idp = TestIdp::new();                                          // in-process RS256 signer + JWKS
let state = harness::auth_state(MemStore::default(), idp.clone()); // AppState with Auth over MemStore
let tok = idp.id_token(Claims::for_sub("g-1").aud("other-client"));
let r = post_json(&state, "/auth/google/onetap", json!({"credential": tok})).await;
assert_eq!((r.status(), r.body_json()), (401, json!({"error": "sign-in failed"})));

// tests/store_contract.rs: one suite, three engines
contract::run(MemStore::default()).await;
contract::run(SqlStore::new(engines::pgsqlite().await?.migrated_pool().await?)).await;
contract::run(SqlStore::new(engines::postgres().await?.migrated_pool().await?)).await;
```

Public surface of `auth`: `Auth::{new, onetap, start, callback, exchange, viewer, logout,
delete_account}`, the `Store` and `Idp` traits with their record types, `SqlStore`, `MemStore`,
`GoogleIdp`, `TestIdp` (feature), the extractors `SignedIn` and `AdminUser`, and the pure `rules`.

## Shape

### Data structures (the core is value types; time and randomness come in as arguments)

| Type | Holds | Invariant encoded |
|---|---|---|
| `Hash32([u8; 32])` | SHA-256 of a secret | only built by `Hash32::of(&[u8])`; the store takes no raw secret |
| `Secret32` | session id, desktop code (32 random bytes) | no `Debug`/`Display` of the value; `.hash()`, `.to_b64()` only |
| `Profile` | verified `sub`, email, `email_verified`, name, picture | only produced by `rules::verify_id_token` |
| `User`, `SessionRow`, `CodeRow` | one row each, typed (`Uuid`, `DateTime<Utc>`) | domain records, no sqlx types leak |
| `SignIn` | profile, new user id, new session, optional new code, `now` | everything the store writes is decided before the call |
| `OAuthCookie` | state, nonce, verifier, client, `dk_hash`, exp | sealed and opened only with `MacKey`; `open` checks MAC then exp |
| `JwksCache` | key set, `fresh_until`, `last_fetch` | pure `plan(kid, now)`; the once-per-minute rule lives in one fn |
| `Seeds` | state, nonce, verifier, session, code bytes, user id (v7) | the only randomness; `Seeds::fresh()` in the shell, fixed in tests |
| `SetCookie` | a `__Host-` cookie | `SetCookie::host(name, value, max_age)` always emits `Secure; HttpOnly; SameSite=Lax; Path=/`, never `Domain` |
| `Viewer` | user, admin, session hash, optional refresh cookie | produced once per request by the session layer |

Dominant access patterns, traced: (a) every authenticated request: cookie, `Hash32::of`, `store.session`
(two single-table reads by primary key), `rules::session_check(row, now)`, at most one `store.slide`
per 24 h; (b) sign-in: verify (cache hit is pure, miss is one fetch), one `store.sign_in` transaction;
(c) admin poll every 1.5 s: path (a) plus `rules::is_admin`, no extra query.

### Module map (`crates/hub-core/src/`, grouped by knowledge; any flow is at most three files)

| File | Owns |
|---|---|
| `config.rs` (M) | `AuthMode::Google(Arc<AuthConfig>)`, required vars, `IdpUrls` (overridable only under `test-idp`) |
| `db.rs` (C) | `connect`, `MIGRATIONS` (embedded), pure `plan_migrations`, `migrate` |
| `auth/mod.rs` (C) | `Auth` (the shell: holds store, idp, keys, cache, clock), `AuthError`, the session layer, extractors |
| `auth/rules.rs` (C) | every sign-in rule as a pure fn, `Refusal`, `OAuthCookie`, `JwksCache`, `Seeds`, `SetCookie` |
| `auth/store.rs` (C) | `Store` trait, records, `MemStore` |
| `auth/sql.rs` (C) | `SqlStore`, the `SQL` statement table (the only SQL text in the server) |
| `auth/idp.rs` (C) | `Idp` trait, `KeyRing`, `GoogleIdp` (reqwest), JWKS JSON parse, `jwks_ttl` |
| `auth/test_idp.rs` (C, `test-idp`) | `TestIdp` (rsa keygen once per process, signs tokens, serves `Idp`) and `/test-idp/*` routes for the 03-03 browser check |
| `routes/auth.rs`, `routes/me.rs` (C) | handlers; `routes/logs.rs` reused for `/admin/logs*` |
| `net.rs` (M) | `same_origin` layer (D-010); rate limiter also covers `/auth/` and `/admin/` |

### Type sketch

```rust
// config.rs
pub enum AuthMode { Off, Google(Arc<AuthConfig>) }
pub struct AuthConfig {
    pub database_url: Redacted<String>, pub client_id: String, pub client_secret: Redacted<String>,
    pub secret_key: Redacted<Vec<u8>>,  // base64url, >= 32 bytes
    pub admin_emails: Vec<String>,      // trimmed, lowercased, empty by default
    pub idp: IdpUrls,                   // Google constants; env overrides only #[cfg(feature = "test-idp")]
}
pub struct IdpUrls { pub auth: String, pub token: String, pub jwks: String, pub issuers: Vec<String> }
pub enum ConfigError { Invalid { key: &'static str, value: String }, Missing(Vec<&'static str>), AuthMismatch }
// PUBLIC_ORIGIN must be https:// with auth on (test-idp builds also accept http://127.0.0.1:<port>).

// db.rs
pub struct Migration { pub version: i64, pub name: &'static str, pub up: &'static str, pub down: &'static str }
pub static MIGRATIONS: &[Migration] = &[Migration { version: 1, name: "users",
    up: include_str!("../../../migrations/0001_users.up.sql"),
    down: include_str!("../../../migrations/0001_users.down.sql") }];
pub fn checksum(sql: &str) -> [u8; 32] { todo!() }          // SHA-256 of the text with CRLF -> LF
/// Pure: what to apply, given what the table says. Refuses a changed checksum, a version the binary
/// does not know (database newer than code), and an out-of-order hole.
pub fn plan_migrations<'m>(all: &'m [Migration], applied: &[(i64, Vec<u8>)]) -> Result<Vec<&'m Migration>, MigrateError> { todo!() }
pub async fn connect(url: &Redacted<String>) -> Result<PgPool, sqlx::Error> { todo!() }
/// Ensures schema_migrations, reads it, applies each planned file in its own transaction
/// (raw_sql(up) then the INSERT), commits. Running it twice applies nothing the second time.
pub async fn migrate(pool: &PgPool, now: DateTime<Utc>) -> Result<Vec<i64>, MigrateError> { todo!() }
pub enum MigrateError { Db(sqlx::Error), ChecksumChanged(i64), UnknownVersion(i64), Gap(i64) }

// auth/store.rs
pub type Fut<'a, T> = futures_util::future::BoxFuture<'a, Result<T, StoreError>>;
pub trait Store: Send + Sync + 'static {
    /// Upsert identity and user, set last_login_at, insert the session and (desktop) the code. One tx.
    fn sign_in(&self, s: SignIn) -> Fut<'_, User>;
    /// The session row and its user (two reads); None if either is missing.
    fn session(&self, id: Hash32) -> Fut<'_, Option<(SessionRow, User)>>;
    fn slide(&self, id: Hash32, expires_at: DateTime<Utc>, last_seen_at: DateTime<Utc>) -> Fut<'_, ()>;
    fn end_session(&self, id: Hash32) -> Fut<'_, ()>;                  // idempotent; cascades codes
    /// Reads and deletes the code in one tx. Some only for the caller whose DELETE removed the row,
    /// so a code is consumed exactly once even under concurrent exchanges, whatever the decision.
    fn take_code(&self, code: Hash32) -> Fut<'_, Option<CodeRow>>;
    /// Insert `new` for the user of `old`, delete `old`. One tx. None if `old` is gone.
    fn rotate_session(&self, old: Hash32, new: NewSession) -> Fut<'_, Option<User>>;
    fn delete_user(&self, id: Uuid) -> Fut<'_, ()>;                    // idempotent; cascades all
    fn sweep(&self, now: DateTime<Utc>) -> Fut<'_, u64>;               // expired sessions (and codes)
}
pub struct SignIn { pub profile: Profile, pub new_user_id: Uuid, pub session: NewSession, pub code: Option<NewCode>, pub now: DateTime<Utc> }
pub struct NewSession { pub id_hash: Hash32, pub created_at: DateTime<Utc>, pub expires_at: DateTime<Utc>, pub user_agent: Option<String> }
pub struct NewCode { pub code_hash: Hash32, pub desktop_key_hash: Hash32, pub expires_at: DateTime<Utc> }
pub enum StoreError { Unavailable(String) }   // conflicts are resolved inside, never surfaced
#[derive(Default)] pub struct MemStore { db: Mutex<MemTables>, writes: AtomicU64, fail_next: AtomicBool }

// auth/idp.rs
pub struct KeyRing { pub keys: KeySet, pub ttl: Duration }            // ttl already clamped 5 min..24 h
pub trait Idp: Send + Sync + 'static {
    fn fetch_jwks(&self) -> BoxFuture<'_, Result<KeyRing, IdpError>>;
    /// Code + PKCE verifier + redirect_uri -> the id_token string (client secret held by the impl).
    fn exchange_code<'a>(&'a self, code: &'a str, verifier: &'a str, redirect_uri: &'a str) -> BoxFuture<'a, Result<String, IdpError>>;
}
pub fn parse_jwks(body: &[u8]) -> Result<KeySet, IdpError> { todo!() }   // RSA, use=sig, alg RS256 or absent
pub fn jwks_ttl(cache_control: Option<&str>) -> Duration { todo!() }      // max-age clamped; absent -> 5 min

// auth/rules.rs (pure; every fn is unit-tested on its own)
pub struct Expect<'a> { pub client_id: &'a str, pub issuers: &'a [String], pub nonce: Option<&'a str> }
/// RS256 only (alg in the header must be RS256, so HS256/none confusion is refused before keys are
/// looked up); signature via jsonwebtoken with its own time checks off; then aud, iss, exp > now,
/// nonce (when expected), sub and email present. Unknown kid -> Refusal::UnknownKid.
pub fn verify_id_token(token: &str, keys: &KeySet, x: &Expect<'_>, now: DateTime<Utc>) -> Result<Profile, Refusal> { todo!() }
pub enum Plan { Use, Fetch }
impl JwksCache {
    /// Fetch when empty or stale; when the kid is unknown, fetch only if the last fetch (ok or
    /// failed) is at least 60 s old. A failed fetch keeps the stale set usable.
    pub fn plan(&self, kid: &str, now: DateTime<Utc>) -> Plan { todo!() }
    pub fn install(&mut self, ring: KeyRing, now: DateTime<Utc>) { todo!() }
    pub fn fetch_failed(&mut self, now: DateTime<Utc>) { todo!() }
}
pub enum SessionCheck { Expired, Valid, Slide { expires_at: DateTime<Utc>, last_seen_at: DateTime<Utc> } }
/// Expired at now >= expires_at; Slide when now - last_seen_at >= 24 h (new expiry now + 30 d).
pub fn session_check(row: &SessionRow, now: DateTime<Utc>) -> SessionCheck { todo!() }
pub enum Client { Web, Desktop }
/// The authorization URL (state, nonce, S256 challenge, redirect_uri = PUBLIC_ORIGIN/auth/google/callback)
/// and the sealed cookie value. Desktop needs dk (22..=128 chars of [A-Za-z0-9_-]); its hash goes in.
pub fn begin(client: Client, dk: Option<&str>, seeds: &Seeds, ac: &AuthConfig, origin: &str, key: &MacKey, now: DateTime<Utc>) -> Result<(String, String), Refusal> { todo!() }
impl OAuthCookie {
    pub fn seal(&self, key: &MacKey) -> String { todo!() }                     // b64url(json) "." b64url(hmac)
    pub fn open(raw: &str, key: &MacKey, now: DateTime<Utc>) -> Result<OAuthCookie, Refusal> { todo!() }
}
pub fn check_state(c: &OAuthCookie, state: &str) -> Result<(), Refusal> { todo!() }  // constant time
/// Taken row (already consumed) + presented key: expiry, then SHA-256(dk) == stored, constant time.
pub fn decide_exchange(row: Option<CodeRow>, desktop_key: &str, now: DateTime<Utc>) -> Result<Hash32, Refusal> { todo!() }
/// email_verified && lowercase(email) in ADMIN_EMAILS.
pub fn is_admin(user: &User, admins: &[String]) -> bool { todo!() }
pub enum Refusal { Malformed, AlgNotRs256, UnknownKid, BadSignature, WrongAudience, WrongIssuer, Expired,
    NonceMismatch, MissingClaim(&'static str), CookieMissing, CookieMac, CookieExpired, StateMismatch,
    BadDesktopKey, CodeUnknown, CodeExpired, WrongDesktopKey }
pub struct MacKey([u8; 32]);   // HKDF-SHA256(salt "vtv", info "oauth-cookie-mac"); "reserved" derived, unused

// auth/mod.rs (the shell: reads the clock once per call, draws Seeds, awaits store and idp)
#[derive(Clone)] pub struct Auth(Arc<Inner>);
pub struct SignedInOk { pub user: User, pub admin: bool, pub cookie: SetCookie }
pub struct Landing { pub location: String, pub cookies: Vec<SetCookie> }  // always clears __Host-vtv_oauth
impl Auth {
    pub fn new(cfg: &Config, store: Arc<dyn Store>, idp: Arc<dyn Idp>, wall: WallClock) -> Result<Auth, ConfigError> { todo!() }
    pub async fn onetap(&self, credential: &str, ua: Option<&str>) -> Result<SignedInOk, AuthError> { todo!() }
    pub fn start(&self, client: Client, dk: Option<&str>) -> Result<Landing, AuthError> { todo!() }
    /// Never fails: any refusal or outage lands on /hub.html?signin=failed (reason logged).
    pub async fn callback(&self, state: Option<&str>, code: Option<&str>, oauth: Option<&str>, ua: Option<&str>) -> Landing { todo!() }
    pub async fn exchange(&self, code: &str, desktop_key: &str, ua: Option<&str>) -> Result<SignedInOk, AuthError> { todo!() }
    pub async fn viewer(&self, session_cookie: Option<&str>) -> Result<Option<Viewer>, AuthError> { todo!() }
    pub async fn logout(&self, session_cookie: Option<&str>) -> Result<SetCookie, AuthError> { todo!() }
    pub async fn delete_account(&self, v: &Viewer) -> Result<SetCookie, AuthError> { todo!() }
}
pub enum AuthError { Refused(Refusal), Idp(IdpError), Store(StoreError), BadRequest(&'static str) }
pub struct SignedIn(pub Viewer);   // extractor: 401 {"error":"signed out"}
pub struct AdminUser(pub Viewer);  // extractor: 404 {"error":"not found"} for signed out and non-admin
```

The desktop flow uses the schema as given and still stores only hashes. The callback cannot hand a
raw session id to the app (the browser is redirected to the deep link), so it creates a placeholder
session whose `expires_at` is the code's 60 s, plus the code row pointing at it. `exchange` calls
`take_code` (consumed exactly once), `decide_exchange`, then `rotate_session`, which inserts a fresh
30-day session (its raw id goes into the webview's cookie) and deletes the placeholder. A refused or
abandoned exchange leaves a row nobody can use, gone in 60 s and swept hourly.

### Router (`app(state)`, still the only router)

`/auth/me` is always mounted (`{"auth": false}` in local mode). Local mode mounts `/api/logs` and
`/api/logs/journal` exactly as today. Auth mode mounts instead two sub-routers, both behind
`net::same_origin` (every non-GET: `Origin` must equal `PUBLIC_ORIGIN`, missing refused, then the
Phase 1 JSON and 300,000-byte checks where there is a body): `signin` (`/auth/google/onetap|start|
callback`, `/auth/desktop/exchange`) and `viewer` (`/auth/me`, `/auth/logout`, `DELETE /api/me`,
`/admin/logs`, `/admin/logs/journal`) with the session layer, which runs `auth.viewer` once, puts
`Option<Viewer>` in the request extensions and appends the slide's refreshed `Set-Cookie`. The path
lists are two `const` arrays (`SIGNIN_ROUTES`, `VIEWER_ROUTES`) used to mount and by the tests, so
a route cannot be added without the tests seeing it. `/api/logs*` with auth on fall to the existing
404 fallback. `/ws` is unchanged in Phase 3 (no consumer of identity before Phase 5).

### SQL: migration 0001 and every statement the store issues

`migrations/0001_users.up.sql` (plain DDL, no defaults, no functions; `.gitattributes` pins LF):

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
CREATE INDEX sessions_expires_at ON sessions (expires_at);
CREATE TABLE desktop_auth_codes (code_hash bytea PRIMARY KEY, desktop_key_hash bytea NOT NULL,
  session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE, expires_at timestamptz NOT NULL);
```

`0001_users.down.sql`: `DROP TABLE desktop_auth_codes; DROP TABLE sessions; DROP TABLE identities;
DROP TABLE users;`. `sessions_expires_at` is an addition to the spec schema (for `sweep`).

| Id | Statement (binds typed from Rust) | Used by |
|---|---|---|
| M1 | `CREATE TABLE IF NOT EXISTS schema_migrations (version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL, applied_at timestamptz NOT NULL)` | migrate |
| M2 | `SELECT version, checksum FROM schema_migrations` | migrate |
| M3 | `INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)` | migrate, same tx as the file |
| S1 | `SELECT user_id FROM identities WHERE provider = $1 AND subject = $2` | sign_in |
| S2 | `INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)` | sign_in (S1 empty) |
| S3 | `INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (provider, subject) DO NOTHING` | sign_in, then S1 again |
| S4 | `DELETE FROM users WHERE id = $1` | sign_in (lost the race: drop our new user), delete_user |
| S5 | `UPDATE users SET email = $2, email_verified = $3, name = $4, picture = $5, last_login_at = $6 WHERE id = $1` | sign_in (existing or race winner) |
| S6 | `INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)` | sign_in, rotate_session |
| S7 | `INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)` | sign_in (desktop) |
| S8 | `SELECT id, email, email_verified, name, picture, created_at, last_login_at FROM users WHERE id = $1` | sign_in, session, rotate_session |
| S9 | `SELECT id_hash, user_id, created_at, expires_at, last_seen_at, user_agent FROM sessions WHERE id_hash = $1` | session, rotate_session |
| S10 | `UPDATE sessions SET expires_at = $2, last_seen_at = $3 WHERE id_hash = $1` | slide |
| S11 | `DELETE FROM sessions WHERE id_hash = $1` | end_session, rotate_session (old) |
| S12 | `SELECT code_hash, desktop_key_hash, session_id_hash, expires_at FROM desktop_auth_codes WHERE code_hash = $1` | take_code |
| S13 | `DELETE FROM desktop_auth_codes WHERE code_hash = $1` | take_code (rows_affected == 1 wins) |
| S14 | `DELETE FROM sessions WHERE expires_at < $1` | sweep |

`sign_in` in one transaction: S1; if found S5; else S2, S3, S1 (if the identity now names another
user, S4 on ours, then S5 on the winner); S6; S7 when desktop; S8; commit. Postgres serializes the
racing S3 on the primary key (the loser's DO NOTHING waits for the winner's commit, read committed
then sees it); pgsqlite has one writer. No SQLSTATE parsing is needed anywhere, because the only
conflict that can happen (two first sign-ins) is absorbed by S3 plus S1, and 256-bit hash
collisions are not a case. The test `sql_shape` scans the `SQL` table and fails on `RETURNING`,
`JOIN`, `(SELECT`, `::`, `CAST`, `DEFAULT`, `now(`, or a statement naming two schema tables.

### Error mapping (every API response keeps `Content-Type: application/json`, `Cache-Control: no-store`)

| Source | Status and body | Logged (target `auth`) |
|---|---|---|
| `same_origin`: Origin missing or not `PUBLIC_ORIGIN` | 403 `{"error":"forbidden origin"}` | no |
| non-JSON body / over 300,000 bytes / bad JSON | 415 `JSON only` / 413 `too large` / 400 `bad request` | no |
| `Refusal` on onetap or exchange | 401 `{"error":"sign-in failed"}` | the variant, never the token |
| `Refusal` or any error in callback | 302 `/hub.html?signin=failed`, oauth cookie cleared | the variant |
| `start` with bad `client` or `dk` | 400 `{"error":"bad request"}` | no |
| `IdpError` (JWKS or token endpoint down) on onetap or exchange | 503 `{"error":"sign-in unavailable"}` | the cause |
| `StoreError` anywhere (session layer included) | 503 `{"error":"sign-in unavailable"}`; signed-out is never faked | the cause |
| `SignedIn` rejection (`DELETE /api/me` signed out) | 401 `{"error":"signed out"}` | no |
| `AdminUser` rejection (signed out, non-admin, unverified email) | 404 `{"error":"not found"}` | no |
| `/api/logs*` with auth on, `/admin/*` in local mode | 404 from the existing fallback | no |
| `/auth/logout` signed out | 204, cookie cleared (idempotent) | no |
| Start-up: missing vars / bad key / http origin / migration | exit 1, `hub-server: missing DATABASE_URL, VTV_SECRET_KEY` / `MigrateError` text | stderr |

### Load-bearing decisions

1. **Rules are pure, the shell is one struct.** Every time and crypto rule takes `now` and `Seeds`,
   so the edge cases are table tests with no harness (per boundary-discipline). `Auth` is the only
   place that reads the clock, draws randomness, and awaits the store or the IdP.
2. **The store speaks domain operations, not tables.** Eight methods, each one transaction-sized
   effect the rules already decided; callers never sequence statements, and the D-014 choreography
   lives once, in `sql.rs` (deep module, per interface depth). `MemStore` makes route tests run
   under the paused clock; the contract suite keeps it honest against both engines.
3. **Single source per invariant.** Auth on or off is `state.auth.is_some()`, and `AppState::build`
   refuses a config whose mode disagrees, so logs cannot be public with auth on. Route lists are
   consts shared by router and tests. The SQL text is one table checked by `sql_shape`.
4. **Idempotent transitions.** Migrate twice applies nothing; logout and delete twice are 204;
   `take_code` consumes on first read regardless of the verdict, so a replay or a wrong key burns the
   code (per make-operations-idempotent).
5. **What it deliberately does not do:** no CSRF token, no `sqlx::migrate!`, no SQLSTATE matching,
   no identity on `/ws`, no `/api/health` database probe (see open questions), no cookie crate.

## Synthesis decision

To be filled by the orchestrator.

## Tradeoffs accepted

- We accept a second storage implementation (`MemStore`, about 150 lines) in exchange for route and
  rule tests that run in milliseconds under the paused clock, with the contract suite as the price
  of keeping it equal to SQL.
- We accept boxed futures on `Store` and `Idp` (`BoxFuture`, from the existing `futures-util`) in
  exchange for `Arc<dyn Store>` in `AppState` without generics leaking into every handler or an
  `async-trait` crate.
- We accept a placeholder session row per desktop sign-in in exchange for keeping the spec schema
  and storing no raw session id anywhere.
- We accept no lock around migrations (pgsqlite has no advisory lock): a second concurrent migrator
  loses on the M3 primary key and rolls back, which is fine for one server process.
- We accept a tokio `Mutex` held across the JWKS fetch in exchange for single-flight refetches; sign-ins
  are rare, and the cache hit path holds it for microseconds.
- We accept a stricter Origin rule on `/admin/logs/journal` than Phase 1's `post_guard` (D-010 wins).

## Alternatives considered

- **Free functions over `&PgPool` with no trait** (tests always on pgsqlite): smaller surface, but
  every route test needs a database process and sockets, so none can use the paused clock (L-016), and
  the 24 h and 30-day edges become wall-clock waits or injected timestamps threaded through SQL. It
  exposes transaction sequencing to handlers. Lost on test reach and interface depth.
- **A per-table repository (`UsersRepo`, `SessionsRepo`, ...)**: mirrors D-014 literally, but callers
  coordinate several methods inside one transaction, so the transaction type leaks to the shell
  (shallow module, temporal decomposition). Lost.
- **Rules as methods on `Auth` with an injected clock trait**: fewer types, but every rule test
  builds an `Auth` with a fake store and IdP to check one comparison. Lost on reader load.

## Open questions and risks

- **Risk 1: `MemStore` drift.** Route tests prove behaviour on a store that is not the one that
  ships. Is the contract suite (every `Store` method, cascades, the race, single consumption) enough,
  or should one end-to-end route test per flow also run on pgsqlite in CI?
- **Risk 2: pgsqlite `--in-memory` and `rows_affected`.** `take_code`'s exactly-once rests on S13's
  `rows_affected` and the race test needs a pool larger than one connection; neither was in the
  D-014 probe, and an in-memory SQLite may be per connection. 03-01's first task should probe both;
  the fallback is a temp-dir database file deleted by the harness (bends L-008) or pool size 1.
- Does the Tauri webview's `POST /auth/desktop/exchange` send `Origin: PUBLIC_ORIGIN` (the app loads
  the hosted page), or `tauri://localhost`, which `same_origin` would refuse?
- How does `auth.js` learn `GOOGLE_CLIENT_ID` for One Tap? Proposal: `/auth/me` adds `"client_id"`
  when `auth` is true (additive to the R4 shape).
- Feature unification: `hub-core`'s self dev-dependency enables `test-idp` for tests; CI should
  assert `cargo tree -p hub-server -e features --edges normal` has no `test-idp`. Acceptable?
- Should `/api/health` report database readiness in auth mode (spec item 20), or wait for Phase 4?

## Plan boundaries

- **03-01 config, db, migrations, store.** `config.rs` (`AuthMode::Google`, `Missing(Vec)`, key and
  origin checks, `test-idp` overrides; `AppState::build` still refuses Google until 03-02);
  `migrations/0001_users.{up,down}.sql`, `.gitattributes`; `db.rs`; `auth/store.rs` with `MemStore`;
  `auth/sql.rs`; `tests/common/engines.rs` (spawns `PGSQLITE_BIN --in-memory --port <free>` with
  `kill_on_drop`; Postgres from `VTV_TEST_POSTGRES_URL`, one database per test); `tests/db_engines.rs`,
  `tests/store_contract.rs`, `sql_shape`. CI job adds Postgres 17 and pgsqlite. Crates: sqlx 0.8
  (`default-features = false`, `runtime-tokio`, `postgres`, `uuid`, `chrono`), uuid 1 (`v7`), chrono
  0.4 (`clock`, `serde`), sha2 0.10.
- **03-02 auth and routes.** `auth/{mod,rules,idp,test_idp}.rs`, `routes/{auth,me}.rs`, `/admin/logs*`,
  `net::same_origin`, limiter prefixes, `app()` mode split, `AppState::build(cfg, auth)`, main wiring,
  hourly `sweep` in `spawn_background`, `tests/auth.rs`, `tests/admin.rs`. Crates: jsonwebtoken 9,
  hmac 0.12, hkdf 0.12, base64 0.22, rand 0.8, subtle 2, rsa 0.9 (optional, `test-idp` only, with
  `[profile.dev.package.num-bigint-dig] opt-level = 3` so keygen stays under a second).
- **03-03 frontend.** `hub/auth.js` (One Tap with the `sessionStorage` flag, header button, avatar
  menu, sign out, delete account dialog), `hub.html` header control, `log_panel.js` asks `/auth/me`
  once and polls `/api/logs`, `/admin/logs` or nothing; tokens only, no native `<select>`; browser
  check against `hub-server --features test-idp` using `/test-idp/*` (authorize auto-approves and
  redirects back, token, JWKS), GIS script blocked by route interception so Google is never called.

## Test strategy

Levels: **R** pure rule unit test (in `rules.rs`, no async); **M** route test over `MemStore` and
`TestIdp` via `tower::ServiceExt::oneshot`, paused clock allowed; **C** store contract suite run on
`MemStore`, pgsqlite and Postgres 17 (C tests use explicit timestamps, real clock, never paused).
`VTV_REQUIRE_ENGINES=1` (set in CI) turns a missing engine from SKIPPED into a failure. Each check
names the mutation it must catch (L-003); 03-02 records each mutation run in its summary.

| Case (item 23 and the facts block) | Proof | Mutation that must fail it |
|---|---|---|
| Wrong `aud`, expired (`exp == now` too), bad signature, wrong `iss`, HS256 and `none` alg | R on `verify_id_token`; M: onetap answers 401 `sign-in failed` | drop each check in turn; use `>=` for exp |
| Wrong or missing `nonce` on callback | R; M: callback lands on `?signin=failed` | skip nonce when `Expect.nonce` is Some |
| First sign-in creates one user, second reuses it, profile updated | C (count users = 1, email changed, `last_login_at` moved); M via onetap twice | remove S1 lookup (two users) |
| Two concurrent first sign-ins, same `sub` | C: `join!` two `sign_in`, one user, one identity, both sessions valid | remove the post-S3 re-read |
| CSRF (now D-010): every non-GET in both const lists with no Origin, wrong Origin, `http://127.0.0.1:<port>` | M table test: 403 `forbidden origin`; correct Origin passes | remove the `same_origin` layer; allow missing Origin |
| Desktop code reuse, expiry (advance 61 s), wrong `desktop_key`, web-started code | R `decide_exchange`; M: 401, second use 401; C: concurrent `take_code` gives one Some | skip S13; compare raw key; `<` vs `<=` on expiry |
| Desktop exchange yields a working session, placeholder gone | C `rotate_session`; M: cookie then `/auth/me` signed in | skip S11 on old |
| Non-admin gets 404 on every `/admin/*` | M over `VIEWER_ROUTES` admin subset: signed out, non-admin, admin with `email_verified=false`, mixed-case admin email (allowed) | drop `email_verified`; case-sensitive compare |
| Slide at most once per 24 h, refreshed cookie, 30-day expiry | R `session_check` at 23h59m, 24h, 30 d; M: `MemStore.writes` unchanged before 24 h, `Set-Cookie` after; expired cookie signed out and row deleted | slide every request; forget `Set-Cookie` |
| OAuth cookie MAC, expiry 600 s, state, attributes, cleared | R seal/open/tamper/expire; M: start sets `__Host-vtv_oauth` with `Max-Age=600`, callback clears it on success and failure | skip MAC verify; skip clear on failure |
| JWKS TTL clamp, kid refetch at most once per minute, stale set on failure | R on `jwks_ttl`, `JwksCache::plan`; M: `TestIdp.fetches` counter across two unknown-kid tokens 30 s and 61 s apart | drop the 60 s guard; no clamp |
| Logout and `DELETE /api/me` (cascade) | C: delete_user leaves zero identities, sessions, codes; M: 204, cookie `Max-Age=0`, old cookie signed out | remove `ON DELETE CASCADE` in MemStore (C catches the drift) |
| Migrations up, idempotent rerun, checksum change refused, down all | C-style `db_engines.rs` on both engines; R `plan_migrations` (gap, unknown version, CRLF equal) | skip checksum compare; no CRLF normalising |
| Config: each missing var named, short key, http origin, release ignores `GOOGLE_ISSUER` | R on `from_lookup`; a `#[cfg(not(feature = "test-idp"))]` test run in the plain CI pass | read issuer without the cfg gate |
| Local mode unchanged | existing contract tests pass with `build(cfg, None)`; M: `/auth/me` gives `{"auth":false}`, `/admin/logs` 404; with auth on `/api/logs` 404 | mount `/api/logs` in both modes |
| D-014 statement shape | `sql_shape` over `SQL` | add `RETURNING id` to S2 |

## Next implementation step

Probe pgsqlite `--in-memory` with a pool of four through sqlx (shared database across connections,
`rows_affected` on S13, `ON CONFLICT DO NOTHING` via the extended protocol), then write `db.rs` and
`plan_migrations` against the result.
