//! The [`Auth`] service: the flows as the architecture's statement sequences, one transaction
//! each. SQL lives in [`store`], decisions in [`rules`], secrets in [`ids`]; this module keeps
//! only sequencing and the cookie format, so handlers stay thin (architecture decision 1).
//!
//! Cookie strings are built only here (`SetCookie`); refusals log at `warn`, target `auth`,
//! the `Refusal` variant only: never a token, code, cookie value or `desktop_key`.
//!
//! Sequences (03-ARCHITECTURE.md, "Every statement the server issues"):
//! - **Sign-in effect** (at most two attempts): S1; hit: S4 (0 rows: roll back, retry), S7;
//!   miss: S2, S3 (0 rows: a racer won, roll back, retry; S1 then hits); then S5, commit.
//! - **Resolve** (no transaction): S6, S7, `is_admin`, S8 on Slide with a refresh cookie.
//! - **Desktop callback**: the effect with a 60 s placeholder S5, then S11.
//! - **Exchange**: S12 (none: CodeUnknown), S13 (`!= 1`: CodeReused), `decide_exchange`;
//!   refusal: S9 the placeholder, commit, 401; pass: S6, S9, S7, S5 fresh, commit.

use std::sync::Arc;

use axum::http::header::SET_COOKIE;
use axum::http::HeaderValue;
use axum::response::Response;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::google::{memory_id_token, memory_keys, Idp, JwksCache};
use crate::auth::ids::{CodeHash, DesktopCode, Keys, SessionHash, SessionId};
use crate::auth::rules::{
    decide_exchange, is_admin, parse_client, session_check, verify_id_token, Client, Expect,
    OauthTicket, Refusal, SessionCheck, VerifiedIdentity,
};
use crate::auth::test_idp::{ManualClock, TestIdp};
use crate::config::AuthSettings;
use crate::db::{now_micros, Db};

pub mod google;
pub mod ids;
pub mod rules;
pub mod store;

#[cfg(feature = "test-idp")]
pub mod test_idp;

/// How long a session cookie and its row last (30 days).
const SESSION_TTL: Duration = Duration::days(30);
/// A desktop placeholder session lives 60 s; the exchange replaces it at once.
const PLACEHOLDER_TTL: Duration = Duration::seconds(60);
/// The sealed oauth cookie lives 600 s.
const OAUTH_COOKIE_TTL: i64 = 600;
/// The session cookie: `__Host-` (secure context enforced by the browser), HttpOnly, Lax.
const SESSION_COOKIE: &str = "__Host-vtv_session";
/// The sealed oauth ticket cookie, cleared on every landing.
const OAUTH_COOKIE: &str = "__Host-vtv_oauth";
/// The identity provider. Phase 03 serves Google only.
const PROVIDER: &str = "google";

// --- Domain types (the caller's view) ----------------------------------------------

/// A signed-in user, as `/auth/me` and the sign-in responses render it.
#[derive(Clone, Debug, Serialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
}

/// A member: a user, the admin flag, and the session that resolved them.
#[derive(Clone, Debug)]
pub struct Member {
    pub user: User,
    pub admin: bool,
    session: SessionHash,
}

/// What the gate's one resolve call produced.
#[derive(Clone, Debug)]
pub enum Viewer {
    Anonymous,
    Member(Member),
}

impl Viewer {
    pub fn member(&self) -> Option<&Member> {
        match self {
            Viewer::Member(m) => Some(m),
            Viewer::Anonymous => None,
        }
    }
}

/// Cookie headers the service produced. Only `auth` builds one; `attach` puts them on a
/// response, and the gate appends a slide cookie unless the handler already set one.
#[derive(Clone, Debug, Default)]
pub struct SetCookie(Vec<HeaderValue>);

impl SetCookie {
    fn push(&mut self, value: String) {
        if let Ok(h) = HeaderValue::from_str(&value) {
            self.0.push(h);
        }
    }

    pub fn attach(self, resp: Response) -> Response {
        let (mut parts, body) = resp.into_parts();
        for h in self.0 {
            parts.headers.append(SET_COOKIE, h);
        }
        Response::from_parts(parts, body)
    }

    /// Whether the response already sets the session cookie (sign-in, logout, delete win).
    #[allow(dead_code)] // the gate reads it when appending its slide cookie (03-02 Task 3)
    pub(crate) fn sets_session(resp: &Response) -> bool {
        resp.headers()
            .get_all(SET_COOKIE)
            .iter()
            .any(|v| v.to_str().is_ok_and(|s| s.starts_with(SESSION_COOKIE)))
    }

    #[cfg(feature = "test-idp")]
    pub fn values(&self) -> Vec<String> {
        self.0
            .iter()
            .filter_map(|h| h.to_str().ok().map(String::from))
            .collect()
    }
}

/// A successful sign-in: the member and the cookie that names their session.
pub struct SignedIn {
    pub member: Member,
    pub cookie: SetCookie,
}

/// Where the oauth callback lands. The oauth cookie is cleared in all three.
pub enum Landing {
    Web {
        cookies: SetCookie,
    },
    Desktop {
        code: String,
        cookies: SetCookie,
    },
    Failed {
        cookies: SetCookie,
    },
}

/// Why an auth call failed at the handler level (the refusal itself logs, never renders).
#[derive(Debug)]
pub enum AuthError {
    SignInFailed(Refusal),
    SignInRequired,
    BadRequest,
    Unavailable(String),
}

/// Where `now` comes from. Production reads the wall clock; tests advance a manual one
/// (architecture decision 7: injected, not paused; L-016 holds, no paused clock with sockets).
#[derive(Clone)]
pub enum AuthClock {
    System,
    #[cfg(feature = "test-idp")]
    Manual(ManualClock),
}

impl AuthClock {
    fn now(&self) -> DateTime<Utc> {
        match self {
            AuthClock::System => now_micros(),
            #[cfg(feature = "test-idp")]
            AuthClock::Manual(c) => c.now(),
        }
    }

    #[allow(dead_code)] // unix() is a test-harness convenience (03-02 Task 3)
    fn unix(&self) -> i64 {
        self.now().timestamp()
    }
}

/// The Idp and clock behind an [`Auth`].
pub struct AuthDeps {
    idp: Idp,
    clock: AuthClock,
}

impl AuthDeps {
    /// reqwest against the configured Google endpoints, wall clock. `redirect_uri` is
    /// `PUBLIC_ORIGIN/auth/google/callback`.
    pub fn production(s: &AuthSettings, redirect_uri: String) -> AuthDeps {
        AuthDeps {
            idp: Idp::Http {
                http: reqwest::Client::new(),
                auth_url: s.idp.auth_url.clone(),
                token_url: s.idp.token_url.clone(),
                jwks_url: s.idp.jwks_url.clone(),
                client_id: s.client_id.clone(),
                client_secret: s.client_secret.expose().to_string(),
                redirect_uri,
            },
            clock: AuthClock::System,
        }
    }

    /// The in-process Idp and a manual clock: every flow without a socket (L-016).
    #[cfg(feature = "test-idp")]
    pub fn in_memory(idp: &TestIdp, clock: &ManualClock) -> AuthDeps {
        AuthDeps {
            idp: Idp::Memory(idp.clone()),
            clock: AuthClock::Manual(clock.clone()),
        }
    }
}

// --- The service -------------------------------------------------------------------

struct Inner {
    settings: Arc<AuthSettings>,
    public_origin: String,
    db: Db,
    keys: Keys,
    idp: Idp,
    clock: AuthClock,
    jwks: JwksCache,
}

#[derive(Clone)]
pub struct Auth(Arc<Inner>);

/// The callback's query parameters, as the handler reads them.
pub struct CallbackParams<'a> {
    pub state: Option<&'a str>,
    pub code: Option<&'a str>,
    pub error: Option<&'a str>,
    pub ticket: Option<&'a str>,
}

impl Auth {
    pub fn new(
        settings: Arc<AuthSettings>,
        public_origin: &str,
        db: Db,
        deps: AuthDeps,
    ) -> Auth {
        let keys = Keys::derive(settings.secret_key.bytes());
        let jwks = JwksCache::new();
        // The memory Idp never fetches: its keys are installed fresh at build (feature only).
        if let Idp::Memory(idp) = &deps.idp {
            jwks.install(
                memory_keys(idp),
                Duration::hours(24),
                deps.clock.now(),
            );
        }
        Auth(Arc::new(Inner {
            settings,
            public_origin: public_origin.to_string(),
            db,
            keys,
            idp: deps.idp,
            clock: deps.clock,
            jwks,
        }))
    }

    /// `GOOGLE_CLIENT_ID`: public by design, `/auth/me` hands it to `auth.js` (D-017).
    pub fn client_id(&self) -> &str {
        &self.0.settings.client_id
    }

    #[allow(dead_code)] // handlers build the authorize redirect (03-02 Task 3)
    pub(crate) fn public_origin(&self) -> &str {
        &self.0.public_origin
    }

    fn now(&self) -> DateTime<Utc> {
        self.0.clock.now()
    }

    /// Google's authorization URL with `state`, `nonce`, PKCE `S256` (architecture decision:
    /// no DB, the ticket rides the sealed cookie). Errors only on a bad `client`/`dk`.
    pub fn begin_sign_in(
        &self,
        client: Option<&str>,
        dk: Option<&str>,
    ) -> Result<(String, SetCookie), AuthError> {
        let parsed = parse_client(client, dk).map_err(|_| AuthError::BadRequest)?;
        let (state, nonce, verifier) = (
            b64url_random(32),
            b64url_random(32),
            b64url_random(64),
        );
        let now = self.now();
        let (client_for_ticket, dk_hash) = match &parsed {
            Client::Web => ("web".to_string(), None),
            Client::Desktop(h) => ("desktop".to_string(), Some(*h)),
        };
        let _ = client_for_ticket; // Client enum already carries the hash; keep both in sync
        let ticket = OauthTicket {
            state: state.clone(),
            nonce: nonce.clone(),
            verifier: verifier.clone(),
            client: parsed,
            dk_hash,
            exp: now.timestamp() + OAUTH_COOKIE_TTL,
        };
        let cookie_value = format!("{}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
            OAUTH_COOKIE,
            ticket.seal(&self.0.keys),
            OAUTH_COOKIE_TTL,
        );
        let mut cookies = SetCookie::default();
        cookies.push(cookie_value);

        let idp = &self.0.idp;
        let auth_url = match idp {
            Idp::Http { auth_url, client_id, redirect_uri, .. } => {
                let mut url = format!(
                    "{}?client_id={}&redirect_uri={}&response_type=code&scope=openid%20email%20profile&state={}&nonce={}&code_challenge={}&code_challenge_method=S256",
                    auth_url,
                    urlencode(client_id),
                    urlencode(redirect_uri),
                    urlencode(&state),
                    urlencode(&nonce),
                    urlencode(&ticket.challenge()),
                );
                if matches!(ticket.client, Client::Desktop(_)) {
                    url.push_str("&prompt=consent");
                }
                url
            }
            #[cfg(feature = "test-idp")]
            Idp::Memory(_) => format!(
                "/__test-idp/authorize?code_challenge={}&nonce={}&redirect_uri={}/auth/google/callback",
                urlencode(&ticket.challenge()),
                urlencode(&nonce),
                urlencode(&self.0.public_origin),
            ),
        };
        Ok((auth_url, cookies))
    }
}

fn b64url_random(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).expect("OS random source");
    URL_SAFE_NO_PAD.encode(buf)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Auth {
    /// The session cookie for a fresh session id: `__Host-vtv_session=<43>; HttpOnly; Secure;
    /// SameSite=Lax; Path=/; Max-Age=2592000`. The raw id leaves only here and only in this
    /// header; the row keeps its hash.
    fn session_cookie(&self, id: &SessionId) -> (String, SessionHash) {
        let value = id.encode();
        let header = format!(
            "{}={}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
            SESSION_COOKIE,
            value,
            SESSION_TTL.num_seconds(),
        );
        (header, id.hash())
    }

    fn cleared_oauth_cookie(&self) -> String {
        format!("{OAUTH_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
    }

    /// The sign-in effect (S1, S4-or-S2+S3, S5), at most two attempts. Retries when S4 or S3
    /// report a racer (`rows_affected == 0`); the second attempt re-reads S1, which then hits.
    async fn sign_in_effect(
        &self,
        identity: &VerifiedIdentity,
        user_agent: Option<&str>,
    ) -> Result<(Member, SetCookie), AuthError> {
        let now = self.now();
        let pool = self.0.db.pool();
        for _attempt in 0..2 {
            match store::identity_user(pool, PROVIDER, &identity.subject)
                .await
                .map_err(db_unavailable)?
            {
                Some(existing) => {
                    let updated = store::update_user_profile(
                        pool,
                        existing.user_id,
                        &identity.email,
                        identity.email_verified,
                        identity.name.as_deref(),
                        identity.picture.as_deref(),
                        now,
                    )
                    .await
                    .map_err(db_unavailable)?;
                    if !updated {
                        // The user row was deleted mid-sign-in: retry (attempt 2 re-reads S1
                        // and takes the miss path).
                        continue;
                    }
                    let user = store::user_row(pool, existing.user_id)
                        .await
                        .map_err(db_unavailable)?
                        .ok_or_else(|| {
                            AuthError::Unavailable("user row vanished".to_string())
                        })?;
                    let session_id = SessionId::fresh();
                    let (cookie_header, id_hash) = self.session_cookie(&session_id);
                    store::insert_session(
                        pool,
                        &id_hash,
                        user.id,
                        now + SESSION_TTL,
                        user_agent,
                        now,
                    )
                    .await
                    .map_err(db_unavailable)?;
                    let admin = is_admin(
                        &user.email,
                        identity.email_verified,
                        &self.0.settings.admin_emails,
                    );
                    let member = Member {
                        user: User {
                            id: user.id,
                            email: user.email,
                            name: user.name,
                            picture: user.picture,
                        },
                        admin,
                        session: id_hash,
                    };
                    let mut cookie = SetCookie::default();
                    cookie.push(cookie_header);
                    return Ok((member, cookie));
                }
                None => {
                    let user_id = Uuid::now_v7();
                    store::insert_user(
                        pool,
                        user_id,
                        &identity.email,
                        identity.email_verified,
                        identity.name.as_deref(),
                        identity.picture.as_deref(),
                        now,
                    )
                    .await
                    .map_err(db_unavailable)?;
                    let won = store::insert_identity(
                        pool,
                        PROVIDER,
                        &identity.subject,
                        user_id,
                        now,
                    )
                    .await
                    .map_err(db_unavailable)?;
                    if !won {
                        // A concurrent first sign-in for the same subject won the S3 race:
                        // our user row is inert (nothing points at it) and the retry re-reads
                        // S1, which now hits for us too.
                        continue;
                    }
                    let session_id = SessionId::fresh();
                    let (cookie_header, id_hash) = self.session_cookie(&session_id);
                    store::insert_session(
                        pool,
                        &id_hash,
                        user_id,
                        now + SESSION_TTL,
                        user_agent,
                        now,
                    )
                    .await
                    .map_err(db_unavailable)?;
                    let admin = is_admin(
                        &identity.email,
                        identity.email_verified,
                        &self.0.settings.admin_emails,
                    );
                    let member = Member {
                        user: User {
                            id: user_id,
                            email: identity.email.clone(),
                            name: identity.name.clone(),
                            picture: identity.picture.clone(),
                        },
                        admin,
                        session: id_hash,
                    };
                    let mut cookie = SetCookie::default();
                    cookie.push(cookie_header);
                    return Ok((member, cookie));
                }
            }
        }
        // Both attempts lost the race; the caller maps this to 401 (the client retries).
        Err(AuthError::SignInFailed(Refusal::IdpRefusedCode))
    }

    /// One Tap: verify the credential against the JWKS, then the sign-in effect.
    pub async fn sign_in_with_id_token(
        &self,
        credential: &str,
        ua: Option<&str>,
    ) -> Result<SignedIn, AuthError> {
        let identity = self.verified_identity(credential, None).await?;
        let (member, cookie) = self.sign_in_effect(&identity, ua).await?;
        Ok(SignedIn { member, cookie })
    }

    /// Verify an id token against the cached JWKS, fetching when the pure plan says so.
    /// A fetch outage with no usable key is `Unavailable`; everything else is a `Refusal`.
    async fn verified_identity(
        &self,
        token: &str,
        nonce: Option<&str>,
    ) -> Result<VerifiedIdentity, AuthError> {
        let now = self.now();
        let kid = peek_kid(token).ok_or(AuthError::SignInFailed(Refusal::Malformed))?;
        let known = self.0.jwks.kid_known(&kid);
        if self
            .0
            .jwks
            .plan(now, known)
            .unwrap_or(false)
        {
            match self.0.idp.fetch_jwks().await {
                crate::auth::google::JwksFetch::Ok { keys, ttl } => {
                    self.0.jwks.store(keys, ttl, now);
                }
                crate::auth::google::JwksFetch::Err => {
                    self.0.jwks.store_failed(now);
                    if !known {
                        return Err(AuthError::Unavailable(
                            "jwks endpoint unreachable".to_string(),
                        ));
                    }
                    // A stale key is still usable until the next successful fetch.
                }
            }
        }
        let keys = self.0.jwks.keys();
        let expect = Expect {
            client_id: &self.0.settings.client_id,
            issuers: &self.0.settings.idp.issuers,
            nonce,
        };
        verify_id_token(token, &keys, &expect, now)
            .map_err(AuthError::SignInFailed)
    }

    /// The gate's one call: S6, S7, maybe S8. Bad or unknown cookie -> Anonymous, no write.
    /// A database outage is `Unavailable` (the gate answers 503; signed-out is never faked).
    pub async fn resolve(
        &self,
        session_cookie: Option<&str>,
    ) -> Result<(Viewer, Option<SetCookie>), AuthError> {
        let Some(cookie) = session_cookie else {
            return Ok((Viewer::Anonymous, None));
        };
        let id_hash = SessionHash::of_cookie(cookie);
        let pool = self.0.db.pool();
        let Some(row) = store::session_row(pool, &id_hash)
            .await
            .map_err(db_unavailable)?
        else {
            return Ok((Viewer::Anonymous, None));
        };
        let now = self.now();
        match session_check(row.expires_at, row.last_seen_at, now) {
            SessionCheck::Expired => Ok((Viewer::Anonymous, None)),
            SessionCheck::Valid => {
                let viewer = self.member_for(row.user_id, id_hash).await?;
                Ok((viewer, None))
            }
            SessionCheck::Slide {
                expires_at,
                last_seen_at,
            } => {
                store::slide_session(pool, &id_hash, expires_at, last_seen_at)
                    .await
                    .map_err(db_unavailable)?;
                let viewer = self.member_for(row.user_id, id_hash).await?;
                let mut cookies = SetCookie::default();
                cookies.push(format!(
                    "{}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
                    SESSION_COOKIE,
                    SESSION_TTL.num_seconds(),
                ));
                Ok((viewer, Some(cookies)))
            }
        }
    }

    async fn member_for(&self, user_id: Uuid, session: SessionHash) -> Result<Viewer, AuthError> {
        let Some(user) = store::user_row(self.0.db.pool(), user_id)
            .await
            .map_err(db_unavailable)?
        else {
            return Ok(Viewer::Anonymous);
        };
        let admin = is_admin(
            &user.email,
            user.email_verified,
            &self.0.settings.admin_emails,
        );
        Ok(Viewer::Member(Member {
            user: User {
                id: user.id,
                email: user.email,
                name: user.name,
                picture: user.picture,
            },
            admin,
            session,
        }))
    }

    /// The oauth callback. Never fails: every refusal or outage lands on
    /// `/hub.html?signin=failed` with the oauth cookie cleared and nothing written.
    pub async fn finish_callback(&self, p: CallbackParams<'_>, ua: Option<&str>) -> Landing {
        let mut cookies = SetCookie::default();
        cookies.push(self.cleared_oauth_cookie());
        let fail = || Landing::Failed {
            cookies: cookies.clone(),
        };

        if p.error.is_some() {
            tracing::warn!(target: "auth", "refusal: IdpRefusedCode");
            return fail();
        }
        let (Some(state), Some(code), Some(ticket_raw)) = (p.state, p.code, p.ticket) else {
            tracing::warn!(target: "auth", "refusal: TicketMissing");
            return fail();
        };
        let ticket = match OauthTicket::open(ticket_raw, &self.0.keys, self.now().timestamp()) {
            Ok(t) => t,
            Err(r) => {
                tracing::warn!(target: "auth", "refusal: {r:?}");
                return fail();
            }
        };
        if ticket.state != state {
            tracing::warn!(target: "auth", "refusal: StateMismatch");
            return fail();
        }

        // The token exchange: HTTP posts the form; the memory Idp mints directly.
        let token = match (&self.0.idp, &ticket.client) {
            (Idp::Http { .. }, _) => match self.0.idp.exchange_code(code, &ticket.verifier).await {
                Ok(resp) => resp.id_token,
                Err(()) => {
                    tracing::warn!(target: "auth", "refusal: IdpRefusedCode");
                    return fail();
                }
            },
            #[cfg(feature = "test-idp")]
            (Idp::Memory(idp), _) => {
                let _ = code;
                // The suites drive the memory path by minting; the verifier must have been
                // registered (PKCE is proved on the HTTP path over the loopback socket).
                memory_id_token(
                    idp,
                    &crate::auth::test_idp::Claims::for_sub("g-0"),
                    Some(&ticket.nonce),
                    0,
                )
            }
        };

        let identity = match self.verified_identity(&token, Some(&ticket.nonce)).await {
            Ok(v) => v,
            Err(AuthError::SignInFailed(r)) => {
                tracing::warn!(target: "auth", "refusal: {r:?}");
                return fail();
            }
            Err(AuthError::Unavailable(m)) => {
                tracing::warn!(target: "auth", "unavailable: {m}");
                return fail();
            }
            Err(_) => return fail(),
        };

        match &ticket.client {
            Client::Web => match self.sign_in_effect(&identity, ua).await {
                Ok((_member, session)) => {
                    let mut both = cookies;
                    for v in session.values() {
                        both.push(v);
                    }
                    Landing::Web { cookies: both }
                }
                Err(_) => fail(),
            },
            Client::Desktop(kh) => {
                // Placeholder session (60 s, its raw id discarded at once) + the code row.
                let result = self.desktop_callback_effect(&identity, Client::Desktop(*kh), ua).await;
                match result {
                    Ok(code_value) => Landing::Desktop {
                        code: code_value,
                        cookies,
                    },
                    Err(_) => fail(),
                }
            }
        }
    }

    /// Desktop callback: the identity effect with a 60 s placeholder session (S1, S2/S3 or
    /// S4, S5 placeholder whose raw id is discarded at once, S11).
    async fn desktop_callback_effect(
        &self,
        identity: &VerifiedIdentity,
        key_hash: crate::auth::rules::Client,
        ua: Option<&str>,
    ) -> Result<String, AuthError> {
        let Client::Desktop(stored_key) = key_hash else {
            return Err(AuthError::BadRequest);
        };
        let now = self.now();
        let pool = self.0.db.pool();
        let user_id = match store::identity_user(pool, PROVIDER, &identity.subject)
            .await
            .map_err(db_unavailable)?
        {
            Some(existing) => {
                let updated = store::update_user_profile(
                    pool,
                    existing.user_id,
                    &identity.email,
                    identity.email_verified,
                    identity.name.as_deref(),
                    identity.picture.as_deref(),
                    now,
                )
                .await
                .map_err(db_unavailable)?;
                if !updated {
                    return Err(AuthError::Unavailable("user row vanished".to_string()));
                }
                existing.user_id
            }
            None => {
                let user_id = Uuid::now_v7();
                store::insert_user(
                    pool,
                    user_id,
                    &identity.email,
                    identity.email_verified,
                    identity.name.as_deref(),
                    identity.picture.as_deref(),
                    now,
                )
                .await
                .map_err(db_unavailable)?;
                let won = store::insert_identity(
                    pool,
                    PROVIDER,
                    &identity.subject,
                    user_id,
                    now,
                )
                .await
                .map_err(db_unavailable)?;
                if won {
                    user_id
                } else {
                    // A racer won S3; re-read S1, which must now hit (the code was just
                    // granted, so only a delete racing us could miss, and that is Unavailable).
                    store::identity_user(pool, PROVIDER, &identity.subject)
                        .await
                        .map_err(db_unavailable)?
                        .map(|a| a.user_id)
                        .ok_or_else(|| {
                            AuthError::Unavailable("identity race".to_string())
                        })?
                }
            }
        };
        // Placeholder session: 60 s, its raw id never leaves this function.
        let placeholder = SessionId::fresh();
        store::insert_session(
            pool,
            &placeholder.hash(),
            user_id,
            now + PLACEHOLDER_TTL,
            ua,
            now,
        )
        .await
        .map_err(db_unavailable)?;
        let code = DesktopCode::fresh();
        store::insert_desktop_code(
            pool,
            &code.hash(),
            &stored_key,
            &placeholder.hash(),
            now + PLACEHOLDER_TTL,
        )
        .await
        .map_err(db_unavailable)?;
        Ok(code.encode())
    }

    /// The desktop exchange: S12 (none: CodeUnknown), S13 (`!= 1`: CodeReused),
    /// `decide_exchange`; on pass S6 placeholder, S9, S7, S5 fresh. The code stays burned
    /// whatever the outcome; on refusal the placeholder dies by expiry and the sweep's
    /// cascade removes it.
    pub async fn exchange_desktop_code(
        &self,
        code: &str,
        desktop_key: &str,
        ua: Option<&str>,
    ) -> Result<SignedIn, AuthError> {
        let code_hash = CodeHash::of_code(code);
        let pool = self.0.db.pool();
        let row = store::code_row(pool, &code_hash)
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::SignInFailed(Refusal::CodeUnknown))?;
        let won = store::burn_code(pool, &code_hash)
            .await
            .map_err(db_unavailable)?;
        if !won {
            return Err(AuthError::SignInFailed(Refusal::CodeReused));
        }
        if let Err(r) = decide_exchange(
            &row.desktop_key_hash,
            row.expires_at,
            desktop_key,
            self.now(),
        ) {
            return Err(AuthError::SignInFailed(r));
        }
        let placeholder = row.session_id_hash;
        let ph_row = store::session_row(pool, &placeholder)
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::SignInFailed(Refusal::CodeExpired))?;
        store::delete_session(pool, &placeholder)
            .await
            .map_err(db_unavailable)?;
        let user = store::user_row(pool, ph_row.user_id)
            .await
            .map_err(db_unavailable)?
            .ok_or(AuthError::SignInFailed(Refusal::CodeUnknown))?;
        let now = self.now();
        let session_id = SessionId::fresh();
        let (cookie_header, id_hash) = self.session_cookie(&session_id);
        store::insert_session(
            pool,
            &id_hash,
            user.id,
            now + SESSION_TTL,
            ua,
            now,
        )
        .await
        .map_err(db_unavailable)?;
        let admin = is_admin(&user.email, user.email_verified, &self.0.settings.admin_emails);
        Ok(SignedIn {
            member: Member {
                user: User {
                    id: user.id,
                    email: user.email,
                    name: user.name,
                    picture: user.picture,
                },
                admin,
                session: id_hash,
            },
            cookie: {
                let mut c = SetCookie::default();
                c.push(cookie_header);
                c
            },
        })
    }

    /// Idempotent: S9, and always a `Max-Age=0` cookie.
    pub async fn sign_out(&self, v: &Viewer) -> Result<SetCookie, AuthError> {
        let mut cookies = SetCookie::default();
        cookies.push(format!(
            "{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
        ));
        if let Viewer::Member(m) = v {
            store::delete_session(self.0.db.pool(), &m.session)
                .await
                .map_err(db_unavailable)?;
        }
        Ok(cookies)
    }

    /// S10 (cascades clear identities, sessions and codes); the old cookie reads signed out.
    pub async fn delete_account(&self, v: &Viewer) -> Result<SetCookie, AuthError> {
        let member = match v {
            Viewer::Member(m) => m,
            Viewer::Anonymous => return Err(AuthError::SignInRequired),
        };
        let mut cookies = SetCookie::default();
        cookies.push(format!(
            "{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0"
        ));
        store::delete_user(self.0.db.pool(), member.user.id)
            .await
            .map_err(db_unavailable)?;
        Ok(cookies)
    }

    /// S14: expired sessions go (their codes die by cascade). Returns rows removed.
    pub async fn sweep_expired(&self) -> Result<u64, AuthError> {
        store::sweep_expired(self.0.db.pool(), self.now())
            .await
            .map_err(db_unavailable)
    }
}

/// The `kid` from a token's header, without verifying anything (JWKS lookup only).
fn peek_kid(token: &str) -> Option<String> {
    let header = jsonwebtoken::decode_header(token).ok()?;
    header.kid
}

/// Map any sqlx error to `Unavailable`, cause stripped of the URL password.
fn db_unavailable(e: sqlx::Error) -> AuthError {
    AuthError::Unavailable(db_cause(&e))
}

fn db_cause(e: &sqlx::Error) -> String {
    match e {
        sqlx::Error::Database(d) => d.message().to_string(),
        other => {
            let mut s = other.to_string();
            if let Some(idx) = s.find("://") {
                // Strip credentials from a connect-string leak (URL password stripped).
                if let Some(at) = s[idx..].find('@').map(|a| a + idx) {
                    if let Some(scheme_end) = s[..idx].rfind(':') {
                        s = format!("{}:{}", &s[..scheme_end], &s[at..]);
                    }
                }
            }
            s
        }
    }
}
