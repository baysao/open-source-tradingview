---
phase: 03-auth
type: Architecture
about: "vtradingview"
date: 2026-11-24
sources:
  - .paul/phases/03-auth/03-GROUNDING.md (binding, with its Addendum)
  - .paul/phases/03-auth/sketch/A-deep-auth-service.md
  - .paul/phases/03-auth/sketch/B-pure-core-store-trait.md
  - .paul/phases/03-auth/sketch/C-types-as-gates.md
---

# Phase 03 architecture: database, Google sign-in, admin

Synthesized by `/architect` (arena Phase B) from three independent candidates. Base: candidate A
(one deep `Auth` service), with named grafts from B and C, and repairs for every binding violation
the three shared. The spec's "Phase 3 facts and decisions" block wins over items 19 to 29.

## Problem

Phase 03 gives a server with no accounts a database, Google sign-in (One Tap, PKCE button, desktop
one-time codes), sessions and an admin gate, while local mode stays unchanged with no database
(D-001, D-012). Five facts force the shape. (1) pgsqlite through sqlx decodes only single-table
statements with no `RETURNING`, join, subquery, cast, function or default (D-014), so multi-table
effects are ordered sequences in one transaction, and the rule needs one home. (2) The owner's
pgsqlite drops `IF NOT EXISTS` and gives each `--in-memory` connection its own database (Addendum);
all three candidates assumed both worked. (3) D-010 is defined by path prefix (non-GET under
`/api/me*`, `/auth/*`, `/admin/*` needs `Origin == PUBLIC_ORIGIN`, missing refused, unlike
`post_guard`), beside the admin 404, the 24 h slide that re-sets the cookie and the MAC'd oauth
cookie. (4) Tests never call Google, write nothing in the repo (L-008), open no socket under a paused
clock (L-016), yet move time by 60 s, 600 s, 24 h and 30 d on pgsqlite and Postgres 17. (5) `app`
stays the only router, `AppState` fields cheap clones, `AppState::build` I/O-free.

## Usage (caller's view)

README of `hub_core::auth`: "With auth on, `AppState::start` connects, migrates and sets `auth:
Some(Auth)`. Give `Auth` a credential, callback parameters or a cookie value; get domain results
carrying their own `SetCookie`. SQL, hashes, cookie format and Google's wire types stay inside.
`routes/gate.rs` applies the Origin, JSON and admin rules before any gated handler runs."

```rust
// hub-server/src/main.rs (03-02): replaces AppState::build
let state = match AppState::start(cfg).await {   // local: build, no I/O; hosted: Db::open + Auth
    Ok(s) => s,
    Err(e) => { eprintln!("hub-server: {e}"); return ExitCode::FAILURE; }  // names the variable or migration
};
let _bg = state.spawn_background();               // + hourly auth.sweep_expired() when hosted

// routes/auth.rs (03-02): One Tap. The gate already checked Origin and JSON.
pub async fn onetap(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let Ok(b) = serde_json::from_slice::<OneTapBody>(&body) else { return bad_request() };
    match auth.sign_in_with_id_token(&b.credential, user_agent(&headers)).await {
        Ok(s) => signed_in(s),             // 200 {"user", "admin"} + Set-Cookie __Host-vtv_session
        Err(e) => e.into_response(),       // 401 {"error":"sign-in failed"}, reason logged
    }
}

// tests/auth.rs (03-02): both engines, no Google, no file in the repo, real tokio time
engines!(fn first_sign_in_creates_one_user(db) {
    let h = Harness::hosted(db).await;                       // TestIdp in memory + ManualClock
    let tok = h.idp.id_token(Claims::for_sub("g-1").email("a@x.io", true));
    let r = h.post("/auth/google/onetap", json!({"credential": tok})).origin_ok().send().await;
    assert_eq!(r.status(), 200);
    h.clock.advance(Duration::from_secs(24 * 3600 + 1));
    assert!(h.get("/auth/me").cookie(&r).send().await.sets_cookie("__Host-vtv_session"));
    assert_eq!(h.db.count("users").await, 1);
});
```

## Shape

### Data structures

```rust
// config.rs (03-01)
#[derive(Clone, Debug)]
pub enum AuthMode { Off, Google(Arc<AuthSettings>) }      // was Copy + PartialEq with Off only
impl AuthMode { pub fn is_local(&self) -> bool { todo!() } }   // lib.rs exempt_loopback, bind_addr
#[derive(Debug)]
pub struct AuthSettings {
    pub database_url: Secret,          // DATABASE_URL
    pub client_id: String,             // GOOGLE_CLIENT_ID (public: auth.js gets it from /auth/me)
    pub client_secret: Secret,         // GOOGLE_CLIENT_SECRET
    pub secret_key: SecretKey,         // VTV_SECRET_KEY, base64url, decoded length >= 32
    pub admin_emails: Vec<String>,     // ADMIN_EMAILS, trimmed, lowercased, empty by default
    pub idp: IdpEndpoints,             // Google's constants; GOOGLE_*_URL, GOOGLE_ISSUER only under test-idp
}
pub struct Secret(String);             // Debug prints "<redacted>"; no Display; .expose() is crate-private
pub struct SecretKey(Vec<u8>);         // Debug redacted; built only by the parser
pub struct IdpEndpoints { pub auth_url: String, pub token_url: String, pub jwks_url: String, pub issuers: Vec<String> }
pub enum ConfigError {
    Invalid { key: &'static str, value: String },  // value printed as "<redacted>" for secret keys
    Missing { keys: Vec<&'static str> },           // every missing name at once, spec order
    AuthNotAvailable,                              // kept by 03-01, deleted by 03-02
}
// Config.public_origin stays the single source: with auth on it must be https:// (test-idp builds
// also accept http://127.0.0.1:<port>). bind_addr() is 127.0.0.1 in both modes.

// db.rs (03-01). A Db value means "connected and migrated" (graft from C).
#[derive(Clone)] pub struct Db(PgPool);            // pool of 5, acquire timeout 5 s
pub struct Migration { pub version: i64, pub name: &'static str, pub up: &'static str, pub down: &'static str }
pub const MIGRATIONS: &[Migration] = &[/* 0001 users via include_str! */];
#[derive(Debug)]
pub enum DbError { Connect(sqlx::Error), Migration { version: i64, source: sqlx::Error },
                   ChecksumChanged { version: i64 }, UnknownApplied { version: i64 }, Gap { version: i64 } }

// auth/ids.rs (03-02). Sealed newtypes (graft from C): fields private, constructors pub(crate).
pub struct SessionId([u8; 32]);        // the secret: no Debug, no Clone; leaves only inside SetCookie
pub struct DesktopCode([u8; 32]);      // the secret one-time code; leaves only in the deep link
// SHA-256 of a SessionId, a DesktopCode, a desktop_key; each Clone + Copy + PartialEq + Eq
pub struct SessionHash([u8; 32]); pub struct CodeHash([u8; 32]); pub struct KeyHash([u8; 32]);
pub(crate) struct Keys { oauth_mac: [u8; 32] }   // HKDF-SHA256 salt "vtv", info "oauth-cookie-mac"; "reserved" derived, unused

// auth/rules.rs (03-02). Pure decisions (graft from B): time and bytes come in as arguments.
pub struct VerifiedIdentity { subject: String, email: String, email_verified: bool,
                              name: Option<String>, picture: Option<String> }  // built only by verify_id_token
pub(crate) struct OauthTicket { state: String, nonce: String, verifier: String,
                                client: Client, dk_hash: Option<KeyHash>, exp: i64 }
pub(crate) enum Client { Web, Desktop(KeyHash) }
pub enum Refusal { Malformed, AlgNotRs256, UnknownKid, BadSignature, WrongAudience, WrongIssuer,
    Expired, NonceMismatch, MissingClaim(&'static str), TicketMissing, TicketBadMac, TicketExpired,
    StateMismatch, IdpRefusedCode, CodeUnknown, CodeReused, CodeExpired, DesktopKeyMismatch }

// auth/mod.rs (03-02)
#[derive(Clone)] pub struct Auth(Arc<Inner>);   // settings, public_origin, db, keys, idp, jwks, clock
pub struct AuthDeps { idp: google::Idp, clock: AuthClock }
pub enum AuthClock { System, #[cfg(feature = "test-idp")] Manual(ManualClock) }
// now() is DateTime<Utc> truncated to microseconds, so a value read back equals the value written.
#[derive(Clone, Debug, Serialize)]
pub struct User { pub id: Uuid, pub email: String, pub name: Option<String>, pub picture: Option<String> }
#[derive(Clone, Debug)] pub struct Member { pub user: User, pub admin: bool, session: SessionHash }
#[derive(Clone, Debug)] pub enum Viewer { Anonymous, Member(Member) }
pub struct SetCookie(Vec<HeaderValue>);          // private constructor: only auth writes cookies
pub struct SignedIn { pub member: Member, pub cookie: SetCookie }
pub enum Landing { Web { cookies: SetCookie }, Desktop { code: String, cookies: SetCookie },
                   Failed { cookies: SetCookie } }  // the oauth cookie is cleared in all three
pub struct CallbackParams<'a> { pub state: Option<&'a str>, pub code: Option<&'a str>,
                                pub error: Option<&'a str>, pub ticket: Option<&'a str> }
pub enum AuthError { SignInFailed(Refusal), SignInRequired, BadRequest, Unavailable(String) }

// routes/table.rs (03-02). One list per mode; app() mounts it and tests enumerate it (graft from B).
#[derive(Clone, Debug)] pub struct RouteSpec { pub method: Method, pub path: &'static str }
```

### Signatures (bodies `todo!()`)

```rust
// db.rs
impl Db {
    /// Connects, then M2 (absent table detected by error, never IF NOT EXISTS), M1 if absent,
    /// plan_migrations, then per pending file: begin, raw_sql(up), M4, commit. A failure re-reads
    /// M2 once (a concurrent starter may have applied it) before giving up.
    pub async fn open(url: &Secret) -> Result<Db, DbError> { todo!() }
    pub(crate) fn pool(&self) -> &PgPool { todo!() }
    #[cfg(feature = "test-idp")] pub async fn open_with(url: &Secret, list: &'static [Migration]) -> Result<Db, DbError> { todo!() }
    #[cfg(feature = "test-idp")] pub async fn migrate_down_all(&self) -> Result<(), DbError> { todo!() }
}
pub fn checksum(sql: &str) -> [u8; 32] { todo!() }    // CRLF -> LF, then SHA-256 (graft from B)
/// Pure: refuses a changed checksum, an applied version this binary lacks, and a hole.
pub fn plan_migrations<'m>(all: &'m [Migration], applied: &[(i64, Vec<u8>)]) -> Result<Vec<&'m Migration>, DbError> { todo!() }
pub fn is_unique_violation(e: &sqlx::Error) -> bool { todo!() }   // 23505, or 42000 + "UNIQUE constraint failed"
fn is_missing_table(e: &sqlx::Error) -> bool { todo!() }          // 42P01, or message contains "no such table"
pub(crate) const STATEMENTS: &[&str] = &[/* M1, M2, M4, M5 */];

// auth/rules.rs
pub(crate) struct Expect<'a> { pub client_id: &'a str, pub issuers: &'a [String], pub nonce: Option<&'a str> }
/// alg must be RS256 before any key lookup; signature via jsonwebtoken with its time checks off;
/// then aud, iss, exp > now, nonce when expected, sub and email present.
pub(crate) fn verify_id_token(token: &str, keys: &KeySet, x: &Expect<'_>, now: DateTime<Utc>) -> Result<VerifiedIdentity, Refusal> { todo!() }
pub(crate) fn jwks_ttl(cache_control: Option<&str>) -> Duration { todo!() }      // max-age clamped 300..=86400, absent 300
/// Fetch when empty or stale; unknown kid fetches only if last fetch (ok or failed) is >= 60 s old.
pub(crate) fn jwks_needs_fetch(fresh_until: Option<DateTime<Utc>>, last_fetch: Option<DateTime<Utc>>, kid_known: bool, now: DateTime<Utc>) -> bool { todo!() }
pub(crate) enum SessionCheck { Expired, Valid, Slide { expires_at: DateTime<Utc>, last_seen_at: DateTime<Utc> } }
pub(crate) fn session_check(expires_at: DateTime<Utc>, last_seen_at: DateTime<Utc>, now: DateTime<Utc>) -> SessionCheck { todo!() }
pub(crate) fn parse_client(client: Option<&str>, dk: Option<&str>) -> Result<Client, Refusal> { todo!() }  // dk: 22..=128 of [A-Za-z0-9_-]
pub(crate) fn decide_exchange(key_hash: &KeyHash, expires_at: DateTime<Utc>, presented: &str, now: DateTime<Utc>) -> Result<(), Refusal> { todo!() }  // expiry, then constant-time compare
pub(crate) fn is_admin(email: &str, email_verified: bool, admins: &[String]) -> bool { todo!() }
impl OauthTicket {
    pub(crate) fn seal(&self, keys: &Keys) -> String { todo!() }          // b64url(json) "." b64url(hmac)
    pub(crate) fn open(raw: &str, keys: &Keys, now: i64) -> Result<OauthTicket, Refusal> { todo!() }  // verify_slice, then exp
    pub(crate) fn challenge(&self) -> String { todo!() }                  // b64url(SHA-256(verifier)), S256
}

// auth/mod.rs
impl AuthDeps {
    pub fn production(s: &AuthSettings) -> AuthDeps { todo!() }                        // reqwest + System
    #[cfg(feature = "test-idp")] pub fn in_memory(idp: &TestIdp, clock: &ManualClock) -> AuthDeps { todo!() }
    #[cfg(feature = "test-idp")] pub fn over_http(s: &AuthSettings, clock: &ManualClock) -> AuthDeps { todo!() }
}
impl Auth {
    pub fn new(settings: Arc<AuthSettings>, public_origin: &str, db: Db, deps: AuthDeps) -> Auth { todo!() }
    pub fn client_id(&self) -> &str { todo!() }
    /// The gate's one call: S6, S7, maybe S8. Bad or unknown cookie -> Anonymous, no write.
    pub async fn resolve(&self, session_cookie: Option<&str>) -> Result<(Viewer, Option<SetCookie>), AuthError> { todo!() }
    pub async fn sign_in_with_id_token(&self, credential: &str, ua: Option<&str>) -> Result<SignedIn, AuthError> { todo!() }
    /// No DB: the authorization URL and the sealed __Host-vtv_oauth cookie (Max-Age=600).
    pub fn begin_sign_in(&self, client: Option<&str>, dk: Option<&str>) -> Result<(String, SetCookie), AuthError> { todo!() }
    /// Never fails: refusals and outages land on /hub.html?signin=failed, reason logged.
    pub async fn finish_callback(&self, p: CallbackParams<'_>, ua: Option<&str>) -> Landing { todo!() }
    pub async fn exchange_desktop_code(&self, code: &str, desktop_key: &str, ua: Option<&str>) -> Result<SignedIn, AuthError> { todo!() }
    pub async fn sign_out(&self, v: &Viewer) -> Result<SetCookie, AuthError> { todo!() }       // idempotent
    pub async fn delete_account(&self, v: &Viewer) -> Result<SetCookie, AuthError> { todo!() }
    pub async fn sweep_expired(&self) -> Result<u64, AuthError> { todo!() }
}
impl SetCookie { pub fn attach(self, resp: Response) -> Response { todo!() }
                 pub(crate) fn sets_session(resp: &Response) -> bool { todo!() } }

// routes/gate.rs: the only place the D-010 and admin rules live
pub(crate) enum Area { Auth, Me, Admin }
/// "/auth/" prefix, "/admin/" prefix, "/api/me" exactly or "/api/me/" prefix (never "/api/metrics").
pub(crate) fn area(path: &str) -> Option<Area> { todo!() }
pub async fn gate(State(st): State<AppState>, req: Request, next: Next) -> Response { todo!() }

// routes/table.rs
pub(crate) fn shared() -> Vec<(RouteSpec, MethodRouter<AppState>)> { todo!() }  // GET /auth/me
pub(crate) fn local() -> Vec<(RouteSpec, MethodRouter<AppState>)> { todo!() }   // /api/logs, /api/logs/journal
pub(crate) fn hosted() -> Vec<(RouteSpec, MethodRouter<AppState>)> { todo!() }  // the 8 hosted routes
pub fn route_specs(state: &AppState) -> Vec<RouteSpec> { todo!() }               // what app(state) mounts from the three

// lib.rs
pub struct AppState { /* Phase 1 and 2 fields */ pub auth: Option<auth::Auth> }
impl AppState {
    pub fn build(cfg: Config) -> Result<AppState, ConfigError> { todo!() }   // local only; Google -> Invalid AUTH_ENABLED "use start"
    pub async fn start(cfg: Config) -> Result<AppState, StartError> { todo!() }
    #[cfg(feature = "test-idp")]
    pub fn build_with_auth(cfg: Config, endpoints: Endpoints, auth: Auth) -> AppState { todo!() }  // panics on AuthMode::Off
}
pub enum StartError { Config(ConfigError), Db(DbError) }
```

### Module map

| File | New or changed | Owns | Plan |
|---|---|---|---|
| `crates/hub-core/migrations/0001_users.up.sql`, `.down.sql` | C | the R3 schema | 03-01 |
| `crates/hub-core/src/config.rs` | M | `AuthMode::Google`, `AuthSettings`, `Secret`, `SecretKey`, `IdpEndpoints`, `Missing`, https origin rule | 03-01 |
| `crates/hub-core/src/db.rs` | C | `Db`, runner, `plan_migrations`, `checksum`, SQLSTATE classifiers, M statements | 03-01 |
| `crates/hub-core/src/lib.rs` | M | `pub mod db` (03-01); `AppState.auth`, `start`, `build_with_auth`, `app()` from the tables, fallback prefixes, sweep task (03-02) | 03-01, 03-02 |
| `crates/hub-core/src/auth/mod.rs` | C | `Auth`, flows as statement sequences, domain types, `AuthError` | 03-02 |
| `crates/hub-core/src/auth/rules.rs`, `auth/ids.rs` | C | pure rules, `Refusal`, `OauthTicket`; sealed secrets and hashes, random, SHA-256, HKDF `Keys` | 03-02 |
| `crates/hub-core/src/auth/store.rs` | C | private: every domain statement, `STATEMENTS` | 03-02 |
| `crates/hub-core/src/auth/google.rs` | C | private: `Idp` enum (`Http`, test-idp `Memory`), JWKS cache state, code exchange | 03-02 |
| `crates/hub-core/src/auth/test_idp.rs` | C (feature) | RS256 signer, JWKS, `/__test-idp/{authorize,token,jwks,mint}` router, `ManualClock` | 03-02 |
| `crates/hub-core/src/routes/{gate,table,auth,me}.rs`; `routes/mod.rs`, `net.rs` | C; M | gate, route lists, thin handlers; module lines, limiter on `/auth/`, `/admin/` | 03-02 |
| `crates/hub-core/Cargo.toml`, `Cargo.toml` | M | crates below, feature `test-idp`, `required-features`, dev profile for `num-bigint-dig` | 03-01, 03-02 |
| `crates/hub-server/src/main.rs`, `Cargo.toml` | M | `AppState::start`; feature `test-idp = ["hub-core/test-idp"]` | 03-02 |
| `crates/hub-core/tests/common/db.rs`, `tests/db_engines.rs` | C | `TestDb`, `engines!`, per-test engines; migrations and engine behaviour | 03-01 |
| `crates/hub-core/tests/{auth,admin,gate}.rs` | C | flows, admin 404, table-driven gate checks | 03-02 |
| `.env.example`, `Procfile`, `scripts/dev.sh`, `.github/workflows/ci.yml` | M | new variables; `db` line; pgsqlite + Postgres 17 jobs, feature run, feature-tree assert | 03-01 (CI feature run 03-02) |
| `hub/auth.js` | C | One Tap, button, avatar menu, sign out, delete account | 03-03 |
| `hub/hub.html`, `hub/log_panel.js` | M | header control and menu; poll only the allowed route | 03-03 |

No `.gitattributes` change (it has `* text=auto eol=lf`); the checksum still normalizes CRLF,
since `include_str!` reads a Windows working copy that may be CRLF.

### Load-bearing decisions

1. **One deep service, thin handlers (A).** Each handler is parse, one `Auth` call, render. The
   sign-in effect is written once and used by One Tap, callback and exchange (per interface depth).
2. **The gate is one middleware keyed by `area(path)`, and the route lists are data (A + B).**
   `routes::gate::gate` is mounted on the whole router when hosted. Order: path outside the three
   areas passes with no DB hit; `/admin/*` resolves the viewer and answers 404 unless admin (before
   the Origin rule, so an outsider learns nothing); any non-GET in an area needs `Origin ==
   PUBLIC_ORIGIN` (missing refused, 403); a non-GET with a body needs JSON (415) under 300,000
   bytes (413); then the viewer is resolved once and inserted as `Extension<Viewer>`; after the
   handler, the slide's `SetCookie` is appended unless the response already sets the session
   cookie (sign-in, logout, delete win). Because the spec defines D-010 by prefix, a prefix gate is
   the faithful shape, and `tests/gate.rs` enumerates `route_specs` so every mounted route is
   checked, and asserts every hosted route lies in an area.
3. **Single-table SQL has one private home, proved by a shape test (A + B + C).** Domain SQL lives
   only in `auth/store.rs`, migration SQL only in `db.rs`. A unit test over both `STATEMENTS` lists
   and the migration files fails when a statement names other than exactly one schema table, or
   contains `RETURNING`, `JOIN`, `(SELECT`, `::`, `CAST(`, `DEFAULT`, `now(` or `IF NOT EXISTS`.
4. **Secrets cannot be stored by mistake (C).** Store functions take `SessionHash`, `CodeHash`,
   `KeyHash`, never the raw values; `SessionId::hash()` is the only producer; `VerifiedIdentity` has
   no public constructor, so the only path to a user row is `verify_id_token`.
5. **Rules are pure (B).** Every time, crypto and admin rule takes `now` and bytes, so each edge
   (`exp == now`, 24 h minus 1 s, 601 s ticket, 59 s and 61 s refetch) is a table test with no
   harness; `auth/mod.rs` keeps only sequencing, which removes A's 500-line file risk.
6. **A `Db` means migrated (C), and `AppState::build` stays I/O-free.** `AppState::start` is the
   one async constructor; `build` refuses a hosted config. Routing derives from
   `state.auth.is_some()`, and only `start` and the test-only `build_with_auth` (which panics on
   `Off`) set it, so logs can never be public with auth on.
7. **Time is injected, not paused (A, C).** `AuthClock::Manual` moves days while pgsqlite runs on a
   real socket under real tokio time (L-016 holds). Tests run every flow on both engines through
   `app(state)`; there is no in-memory store.
8. **The desktop code rotates a placeholder session.** Only hashes are stored, so the callback
   creates a 60 s placeholder session (raw id discarded at once) plus the code row; the exchange
   burns the code, deletes the placeholder and creates the real session. Schema unchanged.

Not done, deliberately: CSRF token (D-010), `sqlx::migrate!` (D-011), compile-time `query!` macros,
`IF NOT EXISTS`, a session lookup on static, chart or `/ws` paths, `/admin/restart`,
`/admin/metrics`, email-based account joining, any database in local mode, an `/api/health` change.

## Migration 0001

```sql
-- crates/hub-core/migrations/0001_users.up.sql (plain DDL: no default, no function, no IF NOT EXISTS)
CREATE TABLE users (id uuid PRIMARY KEY, email text NOT NULL, email_verified boolean NOT NULL,
  name text NULL, picture text NULL, created_at timestamptz NOT NULL, last_login_at timestamptz NOT NULL);
CREATE TABLE identities (provider text NOT NULL, subject text NOT NULL,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE, created_at timestamptz NOT NULL,
  PRIMARY KEY (provider, subject));
CREATE INDEX identities_user_id ON identities (user_id);
CREATE TABLE sessions (id_hash bytea PRIMARY KEY,
  user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE, created_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL, last_seen_at timestamptz NOT NULL, user_agent text NULL);
CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE TABLE desktop_auth_codes (code_hash bytea PRIMARY KEY, desktop_key_hash bytea NOT NULL,
  session_id_hash bytea NOT NULL REFERENCES sessions(id_hash) ON DELETE CASCADE,
  expires_at timestamptz NOT NULL);

-- crates/hub-core/migrations/0001_users.down.sql (indexes go with their tables)
DROP TABLE desktop_auth_codes;
DROP TABLE sessions;
DROP TABLE identities;
DROP TABLE users;
```

`CREATE INDEX` through `raw_sql` on pgsqlite is unmeasured; `db_engines.rs` up on pgsqlite is its
first proof (03-01 task 1).

## Every statement the server issues

All through `sqlx::query(..).bind(..)` or `query_as::<_, (..)>` at runtime; typed binds (`Uuid`,
`DateTime<Utc>`, `Vec<u8>`, `bool`, `Option<String>`, `i64`); provider is bound, not a literal.

| Id | Statement | Table | Used by | Plan |
|---|---|---|---|---|
| M1 | `CREATE TABLE schema_migrations (version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL, applied_at timestamptz NOT NULL)` | schema_migrations | open, only after M2 failed with a missing table | 03-01 |
| M2 | `SELECT version, name, checksum FROM schema_migrations` (sorted in Rust) | schema_migrations | open | 03-01 |
| M3 | the file body via `sqlx::raw_sql` inside the file's transaction | per file | open, down | 03-01 |
| M4 | `INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)` | schema_migrations | open, same tx as M3 | 03-01 |
| M5 | `DELETE FROM schema_migrations WHERE version = $1` | schema_migrations | migrate_down_all (test-idp) | 03-01 |
| S1 | `SELECT user_id FROM identities WHERE provider = $1 AND subject = $2` | identities | sign-in effect | 03-02 |
| S2 | `INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)` | users | first sign-in | 03-02 |
| S3 | `INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (provider, subject) DO NOTHING` | identities | first sign-in; `rows_affected == 0` means a racer won | 03-02 |
| S4 | `UPDATE users SET email = $2, email_verified = $3, name = $4, picture = $5, last_login_at = $6 WHERE id = $1` | users | later sign-in; 0 rows means deleted meanwhile | 03-02 |
| S5 | `INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)` | sessions | sign-in, placeholder, exchange | 03-02 |
| S6 | `SELECT user_id, expires_at, last_seen_at FROM sessions WHERE id_hash = $1` | sessions | resolve, exchange | 03-02 |
| S7 | `SELECT id, email, email_verified, name, picture FROM users WHERE id = $1` | users | resolve, sign-in, exchange | 03-02 |
| S8 | `UPDATE sessions SET expires_at = $2, last_seen_at = $3 WHERE id_hash = $1` | sessions | slide | 03-02 |
| S9 | `DELETE FROM sessions WHERE id_hash = $1` | sessions | sign_out, exchange (placeholder) | 03-02 |
| S10 | `DELETE FROM users WHERE id = $1` | users | delete_account (cascade) | 03-02 |
| S11 | `INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)` | desktop_auth_codes | desktop callback | 03-02 |
| S12 | `SELECT desktop_key_hash, session_id_hash, expires_at FROM desktop_auth_codes WHERE code_hash = $1` | desktop_auth_codes | exchange | 03-02 |
| S13 | `DELETE FROM desktop_auth_codes WHERE code_hash = $1` | desktop_auth_codes | exchange; `rows_affected == 1` wins (measured exact) | 03-02 |
| S14 | `DELETE FROM sessions WHERE expires_at < $1` | sessions | hourly sweep (codes by cascade) | 03-02 |

Tests may also issue `SELECT count(*) FROM <table>` and single-table reads (measured working); they
are never in `STATEMENTS`.

Sequences, one transaction each. **Sign-in effect** (at most two attempts): S1; hit: S4 (0 rows:
roll back, retry), S7; miss: S2 (UUID v7 from Rust), S3 (0 rows: a concurrent first sign-in won,
roll back so our user row goes, retry, where S1 now hits); then S5, commit. **Resolve** (no
transaction, idempotent): parse cookie (43 base64url chars), S6; missing or `expires_at <= now`:
Anonymous; S7; `is_admin`; `session_check` Slide: S8 and a refresh cookie (`Max-Age=2592000`).
**Desktop callback**: the effect with a placeholder S5 (`expires_at = now + 60 s`), then S11.
**Exchange**: S12 (none: CodeUnknown); S13 (`rows_affected != 1`: CodeReused); `decide_exchange`;
on refusal S9 the placeholder, commit (the code stays burned), 401; on pass S6 placeholder, S9, S7,
S5 fresh, commit. S13 runs before S9, or the cascade would empty S13. **Sign out**: S9.
**Delete account**: S10. **Sweep**: S14.

## Error mapping

Every API response keeps `Content-Type: application/json` and `Cache-Control: no-store`
(`routes::json`). Reasons are logged at `warn`, target `auth`, as the `Refusal` variant only: never a
token, code, cookie value or `desktop_key`.

| Source | Status and body | Logged |
|---|---|---|
| gate: `/admin/*` signed out, member, admin email with `email_verified=false` | 404 `{"error":"not found"}` | no |
| gate: non-GET in an area, `Origin` missing or not `PUBLIC_ORIGIN` | 403 `{"error":"forbidden origin"}` | debug |
| gate: non-JSON body / over 300,000 bytes; handler: unparsable JSON | 415 `JSON only` / 413 `too large` / 400 `bad request` | no |
| `AuthError::SignInFailed(r)` on onetap, exchange | 401 `{"error":"sign-in failed"}` | warn, `r` |
| `AuthError::SignInRequired` (`DELETE /api/me` signed out) | 401 `{"error":"sign-in required"}` | no |
| `AuthError::BadRequest` (`start` with unknown `client`, desktop without valid `dk`) | 400 `{"error":"bad request"}` | no |
| `AuthError::Unavailable` (DB error in resolve or a flow; JWKS or token endpoint down with no usable key) | 503 `{"error":"service unavailable"}`; signed-out is never faked | error, cause (URL password stripped) |
| `Landing::Failed` (any refusal or outage in the callback, Google `error=`) | 302 `Location: /hub.html?signin=failed`, oauth cookie cleared | warn |
| `/auth/logout` signed out | 204, cookie `Max-Age=0` (idempotent) | no |
| unknown `/api/*`, `/auth/*`, `/admin/*` (local mode: all hosted routes; hosted: `/api/logs*`) | 404 `{"error":"not found"}` from the fallback | no |
| limiter on `/api/*`, `/auth/*`, `/admin/*` | 429 `{"error":"too many requests"}` + `Retry-After` | no |
| start-up: `ConfigError` / `DbError` | exit 1, `hub-server: missing DATABASE_URL, VTV_SECRET_KEY` / `hub-server: migration 1 (users): <cause>` | stderr |

## Synthesis decision

- **Base: candidate A.** Deepest interface: handlers make one call, SQL is private to one file, and
  every spec case is proven through `app(state)` on both real engines with an injected clock. B's
  route tests never touch pgsqlite; C's handlers compose `idp().verify` then `sign_in` and reach
  `Auth::db()` directly.
- **Grafted from B:** pure `auth/rules.rs` (`verify_id_token`, `session_check`, JWKS plan and TTL,
  `decide_exchange`, `is_admin`, ticket seal and open), which shrinks A's `auth/mod.rs`; route lists
  as data that `app()` mounts and tests enumerate (`routes/table.rs`); pure `plan_migrations` with
  `Gap`; CRLF-normalized checksum; `dk` charset and length; `/auth/me` carrying `client_id`.
- **Grafted from C:** sealed newtypes `SessionId`, `SessionHash`, `DesktopCode`, `CodeHash`,
  `KeyHash` and the constructor-less `VerifiedIdentity`; `Db::open` means migrated, replacing A's
  `new` then `prepare` two-step; `AppState::start` beside a sync, I/O-free `build`; the fallback
  404 extended to `/auth/` and `/admin/`; the shape test's "exactly one schema table" rule; a slide
  cookie never overwrites a cookie the handler set; `VTV_TEST_REQUIRE` turning SKIPPED into failure.
- **Repaired in all three:** `CREATE TABLE IF NOT EXISTS` (A, B, C M1) becomes M2-first with the
  absent-table error, then plain M1; `--in-memory` test engines (A, B, C) become a per-test
  file-backed pgsqlite process in the OS temp directory.
- **Rejected:** B's `Store` trait and `MemStore` (a second home for the D-014 choreography and
  the cascades; its one remaining reason, the paused clock, the manual clock removes); B's and C's
  extractor-based admin gate (a forgotten extractor on a new `/admin/` route is an open door); C's
  `Routes::write` builder with `Proof`, `OriginRule`, `Checked` (the axum 0.8 `Handler` inference
  risk C names, and three gate types every handler author must learn; the gate plus the table test
  gives the same guarantee on plain `Router::route`); C's lazy `ViewerCell` and source-scan
  `route_gate.rs`; B's extra `sessions_expires_at` index (keeps R3 exact); B's `AppState::build(cfg,
  auth)` signature change. The rest are in the table below.

Red-flag findings (per `design-red-flags.md`):

| Candidate | Finding | Where | Disposition |
|---|---|---|---|
| A | Information leakage: `Auth::admit` takes `Method`, path and `HeaderMap`, so route prefixes live in the domain service and in `app()` | A "The operations", decision 1 | gate moved to `routes/gate.rs`; `Auth::resolve` takes a cookie value |
| A | Temporal decomposition: `Auth::new` then `prepare`; a caller can forget the second step | A "The operations", Usage | `Db::open` connects and migrates; `Auth::new` takes a `Db` |
| A | Large module: one `auth/mod.rs` near 500 lines mixing rules and sequencing | A "Tradeoffs" | pure rules split to `rules.rs` |
| B | Shallow public surface: traits, records, `SqlStore`, `MemStore`, `GoogleIdp`, extractors and rules all `pub` | B "Usage", last paragraph | rules `pub(crate)`; store private |
| B | Pass-through risk: a `Store` trait with one shipping implementation; drift named by B itself | B "Open questions" risk 1 | trait rejected |
| B | Information leakage: the D-014 choreography written twice (`sql.rs`, `MemStore`) | B decision 2 | one home: `store.rs` |
| C | Shallow modules: per-table one-statement functions; handlers coordinate `verify` then `sign_in` | C decision 5, `onetap` usage | rejected |
| C | Information leakage: `me::delete` issues `db::users::delete` and knows the cascade; `Auth::db()` exposes the pool | C "Usage" | `Auth::delete_account` |
| C | Pass-through: `admin::logs` and `admin::journal` wrap Phase 1 handlers unchanged | C "Usage" | handlers mounted directly behind the gate |
| A, B, C | Binding: M1 `IF NOT EXISTS` and `--in-memory` engines (C also inlines `'google'`, legal but bound here) | each M1 row and test strategy | repaired, see above |

## Tradeoffs accepted

- We accept `SetCookie` (an opaque `HeaderValue` list) on the `Auth` API in exchange for cookie
  names, attributes and the slide having one home; `HeaderMap` and `Method` stay out of `auth`.
- We accept a path-prefix gate in exchange for one function that matches D-010's own wording; the
  table enumeration test is the guard against a route mounted outside an area.
- We accept two indexed single-table reads (S6, S7) per gated request, no cache, in exchange for no
  invalidation on sign-out or delete; chart, static and `/ws` paths never touch the database.
- We accept that every auth test needs a real engine process, in exchange for no in-memory store to
  drift; pure rules carry the fast edge-case tables.
- We accept a 60 s placeholder session per desktop sign-in in exchange for an unchanged schema and
  no raw session id stored anywhere.
- We accept a 503 on `/admin/*` during a database outage (admin status cannot be decided), in
  exchange for never answering signed-out when the truth is unknown.
- We accept no migration lock (pgsqlite has none): a racing second starter loses on M4's key,
  re-reads M2 once and continues, or fails start-up with the version named.

## Alternatives considered

Runner shapes B and C are judged in the synthesis above; these are shapes the synthesis weighed.

- **A layer per area via nested routers** (`/auth` and `/admin` nests, each with its own layer):
  structural, but `/api/me` sits inside the Phase 1 `/api` space, so the policy splits across three
  layers and the admin-before-Origin order is repeated. Lost on one-home policy.
- **An in-memory session cache keyed by `SessionHash`:** saves S6 and S7 per gated request, but
  sign-out, delete and the sweep must invalidate it, and a second process would see stale rows.
  Lost: the reads are two primary-key lookups.
- **Per-route extractors (`SessionUser`, `AdminUser`, spec item 21):** each handler composes its own
  policy; a missing extractor compiles and passes the happy path. Lost.

## Open questions and risks (each with the recommended default, recorded for the owner)

| Question | Recommended default (owner may override) |
|---|---|
| Does a wrong `desktop_key` burn the code? | Yes: S13 runs before any check, so every attempt spends it. Stops brute force on `desktop_key`; the cost is that someone holding a leaked code can cancel a legitimate sign-in, and the app restarts the flow. Proposed D-015. |
| `/api/health` readiness in Phase 3? | No. Health keeps its Phase 1 parity body; `Db::open` already fails start-up. A readiness field arrives with the Phase 4 deploy. Proposed D-016. |
| How does `auth.js` learn `GOOGLE_CLIENT_ID`? | `/auth/me` adds `"client_id"` whenever `auth` is true (additive to the R4 shape; the id is public by design). No templating of static HTML. Proposed D-017. |
| `test-idp` scope? | One hub-core feature, forwarded by `hub-server`: TestIdp signer and JWKS, `Idp::Memory`, `ManualClock`, `GOOGLE_*` URL and issuer reads, `http://127.0.0.1` origin, `migrate_down_all`, and `/__test-idp/{authorize,token,jwks,mint}` mounted by `app()` only in hosted mode. CI asserts `cargo tree -p hub-server -e features` for the release build has no `test-idp`. |
| `Origin` on `POST /auth/desktop/exchange`? | The app window loads the hosted site, so its Origin is `PUBLIC_ORIGIN` and D-010 applies unchanged; no special case in Phase 3. A Phase 06 check asserts the webview sends it (refused as `tauri://localhost` otherwise). |
| Test-database isolation (Addendum options a, b)? | Option (a): one pgsqlite process per test on a free loopback port over a fresh file in a `tempfile::TempDir` under the OS temp directory, killed and deleted on drop. Tests stay parallel, a panic cannot leave state for the next test, the race tests get a real pool of 4 over one file (measured shared), and it mirrors Postgres's database-per-test. (b) serializes the suite and makes every test depend on the down migration being right. 03-01 measures start-up; if it exceeds 1 s per test the plan records it and keeps (a) unless the suite passes 3 minutes. |
| Concurrent write transactions on file-backed pgsqlite | Unmeasured: two transactions writing at once may get a busy error instead of waiting. 03-01 measures it; the sign-in effect's second attempt also covers a busy error classified as retryable. |
| RSA key generation in debug builds | `rsa 0.9` under `test-idp`, two keys generated once per test binary (`OnceLock`), `[profile.dev.package.num-bigint-dig] opt-level = 3`. No PEM committed (no-secrets rule, scanner noise). |
| Rate limiter on `/auth/*`, `/admin/*`? | Yes, same bucket; admin polling at 1.5 s is far under 20/s. |
| Production `DATABASE_URL` TLS | Not in Phase 3 (pgsqlite on loopback); Phase 4 adds a sqlx TLS feature if the hosted database is remote. |

## Plan boundaries

**03-01: config, db, migrations.** Files: `config.rs`, `db.rs`, `lib.rs` (`pub mod db` only),
`migrations/0001_users.{up,down}.sql`, `tests/common/db.rs`, `tests/db_engines.rs`, `hub-core` and
workspace `Cargo.toml` (feature `test-idp` declared, `db_engines` with `required-features`),
`.env.example`, `Procfile`, `scripts/dev.sh`, `.github/workflows/ci.yml` (pgsqlite from
`PGSQLITE_BIN`, Postgres 17 service, `VTV_TEST_REQUIRE=pgsqlite,postgres`). Crates: `sqlx 0.8`
(`default-features = false`; `runtime-tokio`, `postgres`, `uuid`, `chrono`; no `macros`), `uuid 1`
(`v7`, `serde`), `chrono 0.4` (`default-features = false`; `clock`, `serde`), `sha2 0.10`; dev:
`tempfile 3`. `AppState::build` still refuses hosted mode (`AuthNotAvailable` kept).

| Acceptance check | Mutation that must turn it red (L-003) |
|---|---|
| hosted config names every missing variable at once; `http://` origin and a 31-byte key refused | `Missing` reports only the first; accept http; `>= 31` |
| `GOOGLE_*_URL` ignored in a default-feature build | read them without the `cfg` gate |
| `Debug` of `AuthSettings` shows no secret | derive `Debug` on `Secret` |
| up on empty applies `[1]`; second up applies `[]`, both engines | skip the M2 check (second up fails "already exists") |
| absent `schema_migrations` detected on both engines; a unique-violation error is not "absent" | use `IF NOT EXISTS`; treat any M2 error as absent |
| down all leaves only an empty `schema_migrations`; up again works | drop one `DROP TABLE` from the down file |
| edited checksum refused, unknown applied version refused, gap refused | skip the checksum compare in `plan_migrations` |
| failing second statement in a file leaves no table and no M4 row | run M4 outside the file's transaction |
| CRLF and LF text give one checksum | remove the normalization |
| round trip of `uuid`, `bool`, microsecond `timestamptz`, `bytea`, `NULL` text, both engines | write nanosecond times (no truncation) |
| duplicate primary key: `is_unique_violation` true on both engines | match `23505` only |
| deleting a user removes identities, sessions, codes, both engines | remove one `ON DELETE CASCADE` |
| shape test over `db::STATEMENTS` and migration files | add `IF NOT EXISTS` to M1 |
| test database file lies under `std::env::temp_dir()` and is gone after drop | place it under `CARGO_MANIFEST_DIR` |
| an absent engine fails when `VTV_TEST_REQUIRE` names it | print SKIPPED and pass regardless |
| local suites unchanged (`cargo test --workspace`, fmt, clippy `-D warnings`) | none: regression gate |

Measured into the summary, not gated: pgsqlite start-up per test; concurrent writers on one file.

**03-02: auth and routes.** Files: `auth/{mod,rules,ids,store,google,test_idp}.rs`,
`routes/{gate,table,auth,me,mod}.rs`, `net.rs`, `lib.rs`, `config.rs` (drop `AuthNotAvailable`),
`hub-server/src/main.rs` and `Cargo.toml`, `tests/{auth,admin,gate}.rs` (`required-features =
["test-idp"]`), CI second run `cargo test -p hub-core --features test-idp` plus clippy with the
feature and the feature-tree assert. Crates: `jsonwebtoken 9`, `hmac 0.12`, `hkdf 0.12`, `base64
0.22`, `getrandom 0.2`, `subtle 2`; optional under `test-idp`: `rsa 0.9`. `reqwest 0.12` (existing)
posts the token form. Every row runs on both engines unless marked R (pure unit test).

| Acceptance check | Mutation that must turn it red |
|---|---|
| R: wrong `aud`, wrong `iss`, `exp == now`, bad signature, HS256 and `none`, missing `sub`; onetap 401 `sign-in failed`, users count 0 | drop each check in turn; `>=` for exp; accept any alg |
| wrong or missing `nonce` on callback lands on `?signin=failed` | skip nonce when expected |
| first sign-in creates one user; second reuses it, profile and `last_login_at` updated | always take the S2 path; drop S4 |
| two concurrent first sign-ins (same `sub`, pool of 4): one user, one identity, both 200 | ignore S3's `rows_affected` |
| `tests/gate.rs` over `route_specs`: every hosted non-GET with no Origin and foreign Origin 403, `PUBLIC_ORIGIN` passes; GET `/auth/me` needs none; every hosted route is in an area | let a missing Origin pass; mount a route outside the areas |
| `text/plain` body 415; 300,001 bytes 413 | drop the JSON rule |
| every `/admin/*` from the table: signed out, member, unverified admin email, foreign Origin all 404; admin 200 | ignore `email_verified`; check Origin before admin |
| desktop: start with `client=desktop&dk=K`, callback 302 `vtradingview://auth?code=C`, exchange 200 with cookie, second exchange 401 | drop S13 |
| desktop expiry (+61 s) 401; wrong key 401 then right key 401 (burned); concurrent exchanges one 200 | skip expiry; skip key compare; ignore `rows_affected` |
| placeholder unusable: no response carries its id; its row expires in 60 s and is gone after exchange | give the placeholder 30 d; skip S9 |
| `sessions.id_hash` equals SHA-256 of the cookie value, never the value | store the raw id |
| slide: +23 h no Set-Cookie and row unchanged; +24 h 1 s one Set-Cookie and row moved; +30 d 1 s idle signed out | slide every request; `<` for expiry |
| oauth ticket: start sets `__Host-vtv_oauth` `Max-Age=600`, `S256` challenge; tampered MAC, +601 s, wrong state, absent: failed, cleared each time | skip `verify_slice`; skip exp; keep cookie on failure |
| PKCE: TestIdp checks `S256(verifier) == challenge` | send no verifier |
| JWKS: two tokens one fetch; unknown kid 30 s apart one refetch, 61 s apart two; `max-age=10` clamps to 300 | drop the 60 s guard; no clamp |
| logout 204, `Max-Age=0`, row gone, second logout 204 | drop S9 |
| delete 204; every table count 0 for that user; old cookie signed out | delete only the session |
| local mode: `/auth/me` `{"auth":false}`; hosted routes 404 JSON; `/api/logs*` as Phase 1; hosted: `/api/logs*` 404 | mount the hosted table in local mode |
| HTTP IdP path: `AuthDeps::over_http` against `/__test-idp/*` on a loopback socket, onetap 200 | parse max-age as minutes |
| shape test over `store::STATEMENTS` | add `RETURNING id` to S2 |
| log ring after a full flow holds no cookie, token or code value | log the credential on refusal |

**03-03: frontend, log panel gating, browser check.** Files: `hub/auth.js` (C), `hub/hub.html`
(M: header sign-in control, avatar menu per mockup 01, delete dialog per mockup 04, `auth.js`
loaded), `hub/log_panel.js` (M). No crates. One Tap uses the GIS JavaScript callback posting
`{credential}` as JSON with the `sessionStorage` dismissal flag; the menu is a button plus menu
pattern (no native `<select>`, no one-sided border, tokens only). Browser check: `hub-server
--features test-idp`, `PUBLIC_ORIGIN=http://127.0.0.1:<port>`, GIS script blocked by route
interception and replaced by a stub that calls the callback with a `/__test-idp/mint` token.

| Acceptance check | Mutation that must turn it red |
|---|---|
| local mode: panel polls `/api/logs`; no sign-in control rendered | poll `/admin/logs` always |
| hosted signed out and member: zero `/api/logs` and `/admin/logs` requests in 15 s (network log) | poll unconditionally |
| hosted admin: menu shows the log panel entry; panel polls `/admin/logs`; journal POST 200 | hide the entry for admins |
| button flow through `/__test-idp/authorize` ends signed in with the avatar shown | drop the callback redirect handling |
| One Tap stub: sign-in POST is JSON, prompt not shown again after dismissal in the tab | post form-encoded; ignore the flag |
| sign out and delete account (confirm dialog, keyboard reachable, Escape closes) return to signed out | delete without the confirm |
| light and dark, 320 px and desktop widths, every control in its states | none: visual review recorded with screenshots |

## Next implementation step

03-01 task 1: add `sqlx` and write `tests/common/db.rs` (per-test pgsqlite over a temp-directory
file, per-test Postgres database) and `db.rs` with M1 to M5, then make `db_engines.rs` up, up,
down, up green on both engines, measuring pgsqlite start-up and concurrent writers on the way.
