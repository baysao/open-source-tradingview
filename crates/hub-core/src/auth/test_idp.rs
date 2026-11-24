//! The test identity provider (feature `test-idp` only): a real RS256 signer with a JWKS
//! document, an authorization-code table that checks PKCE, and a manual clock for the suites.
//!
//! Tests never call Google. `TestIdp` mints tokens with chosen claims and `kid`; the
//! [`router`] is mounted by `app()` only in hosted mode under `/__test-idp/`, so the HTTP
//! [`Idp`](crate::auth::google::Idp) path is exercised against a real socket (never under a
//! paused clock: L-016 holds, time is injected by [`ManualClock`]).
//!
//! The RSA keys are generated once per test binary ([`OnceLock`]) and never committed.

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Seconds an issued id token stays valid (the suites move the clock past it only on purpose).
const TOKEN_TTL: i64 = 3600;

/// A manual wall clock the suites advance; every auth decision reads it through `AuthClock`.
#[derive(Clone, Debug)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    /// Starts at `start_unix` seconds.
    pub fn new(start_unix: i64) -> ManualClock {
        ManualClock(Arc::new(AtomicI64::new(start_unix)))
    }

    /// The current instant, microsecond-truncated like `db::now_micros`.
    pub fn now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_micros(self.0.load(Ordering::SeqCst) * 1_000_000)
            .unwrap_or_else(chrono::Utc::now)
    }

    pub fn unix(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Moves the clock forward (`chrono::Duration` so callers can say `hours(24)`).
    pub fn advance(&self, d: chrono::Duration) {
        self.0.fetch_add(d.num_seconds(), Ordering::SeqCst);
    }
}

/// One generated RSA key pair with its `kid`.
struct IdpKey {
    kid: String,
    signing: SigningKey<Sha256>,
    public: RsaPublicKey,
}

/// Two RSA keys per test binary: key 0 signs; key 1 exists so a token with an unknown `kid`
/// (and the JWKS rotation test) has a real second key to name.
struct IdpKeys([IdpKey; 2]);

impl IdpKeys {
    fn get() -> &'static IdpKeys {
        static KEYS: OnceLock<IdpKeys> = OnceLock::new();
        KEYS.get_or_init(|| {
            let mk = |kid: &str| {
                let private =
                    RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).expect("rsa keygen");
                let public: RsaPublicKey = private.clone().into();
                IdpKey {
                    kid: kid.to_string(),
                    signing: SigningKey::<Sha256>::new(private),
                    public,
                }
            };
            IdpKeys([mk("test-key-0"), mk("test-key-1")])
        })
    }
}

/// Chosen token claims. `Default` is a valid Google-shaped token for sub "g-0".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub aud: Option<String>,
    pub iss: Option<String>,
    /// `Some(None)` mints a token without the claim; `None` uses the default.
    pub nonce: Option<Option<String>>,
    /// `Some(None)` mints a token without `exp`; `None` uses now + [`TOKEN_TTL`].
    pub exp: Option<Option<i64>>,
}

impl Claims {
    pub fn for_sub(sub: &str) -> Claims {
        Claims {
            sub: sub.to_string(),
            email: format!("{sub}@example.test"),
            email_verified: true,
            name: Some(format!("User {sub}")),
            picture: None,
            aud: None,
            iss: None,
            nonce: None,
            exp: None,
        }
    }

    pub fn email(mut self, email: &str, verified: bool) -> Claims {
        self.email = email.to_string();
        self.email_verified = verified;
        self
    }

    pub fn aud(mut self, aud: &str) -> Claims {
        self.aud = Some(aud.to_string());
        self
    }

    pub fn iss(mut self, iss: &str) -> Claims {
        self.iss = Some(iss.to_string());
        self
    }

    pub fn nonce(mut self, nonce: Option<&str>) -> Claims {
        self.nonce = Some(nonce.map(|n| n.to_string()));
        self
    }

    pub fn exp(mut self, exp: Option<Option<i64>>) -> Claims {
        self.exp = exp;
        self
    }
}

/// An authorization code handed out by `authorize` and spent at `token`.
struct CodeGrant {
    challenge: String,
    nonce: String,
    redirect_uri: String,
}

/// The in-process identity provider (or behind the router, over HTTP, the same thing).
#[derive(Clone)]
pub struct TestIdp {
    /// `now_unix -> JWKS Cache-Control`.
    cache_control: Arc<Mutex<String>>,
    codes: Arc<Mutex<HashMap<String, CodeGrant>>>,
    /// Counters the JWKS tests read.
    pub jwks_fetches: Arc<AtomicI64>,
    /// Last `code_verifier` the token endpoint saw (the PKCE test reads it).
    pub last_verifier: Arc<Mutex<Option<String>>>,
}

impl Default for TestIdp {
    fn default() -> Self {
        TestIdp::new()
    }
}

impl TestIdp {
    pub fn new() -> TestIdp {
        TestIdp {
            cache_control: Arc::new(Mutex::new("public, max-age=300".to_string())),
            codes: Arc::new(Mutex::new(HashMap::new())),
            jwks_fetches: Arc::new(AtomicI64::new(0)),
            last_verifier: Arc::new(Mutex::new(None)),
        }
    }

    /// Overrides the `Cache-Control` the JWKS endpoint answers with.
    pub fn set_cache_control(&self, value: &str) {
        *self.cache_control.lock().unwrap_or_else(|p| p.into_inner()) = value.to_string();
    }

    /// The raw RS256 signature bytes over the signing input (jsonwebtoken verifies raw
    /// bytes; `Signature::to_string()` would be hex text and would never verify).
    pub fn sign(&self, signing_input: &[u8], kid: usize) -> Vec<u8> {
        let keys = IdpKeys::get();
        keys.0[kid % keys.0.len()]
            .signing
            .sign(signing_input)
            .to_bytes()
            .to_vec()
    }

    /// The `kid` values the JWKS serves, in order.
    pub fn kids(&self) -> [String; 2] {
        let keys = IdpKeys::get();
        [keys.0[0].kid.clone(), keys.0[1].kid.clone()]
    }

    /// Mints an id token signed by `kid` (default 0) with `claims` and the given `nonce`.
    pub fn id_token(&self, claims: &Claims, nonce: Option<&str>, kid: usize) -> String {
        let keys = IdpKeys::get();
        let key = &keys.0[kid % keys.0.len()];
        let now = chrono::Utc::now().timestamp();
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": key.kid});
        let mut payload = json!({
            "iss": claims.iss.clone().unwrap_or_else(|| "https://accounts.google.com".into()),
            "aud": claims.aud.clone().unwrap_or_default(),
            "sub": claims.sub,
            "email": claims.email,
            "email_verified": claims.email_verified,
            "name": claims.name,
            "picture": claims.picture,
            "iat": now,
        });
        // `Some(None)` deliberately omits `exp`; `None` keeps the default TTL.
        if let Some(exp) = claims.exp.unwrap_or(Some(now + TOKEN_TTL)) {
            payload["exp"] = json!(exp);
        }
        // `Some(None)` deliberately omits `nonce`; `None` falls back to the caller's nonce.
        let n = claims
            .nonce
            .clone()
            .or_else(|| nonce.map(|n| Some(n.to_string())));
        if let Some(Some(n)) = n {
            payload["nonce"] = json!(n);
        }
        let b64 = |b: &[u8]| URL_SAFE_NO_PAD.encode(b);
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(payload.to_string().as_bytes())
        );
        let sig = self.sign(input.as_bytes(), kid);
        format!("{input}.{}", b64(&sig))
    }

    /// A token whose header names a kid the JWKS never serves, signed by a real key (the
    /// unknown-kid edge: refused at lookup, before any signature work).
    pub fn id_token_unknown_kid(&self, claims: &Claims, nonce: Option<&str>) -> String {
        let mut c = claims.clone();
        c.iss
            .get_or_insert_with(|| "https://accounts.google.com".into());
        let signed = self.id_token(&c, nonce, 1);
        // Rebuild the header with a kid the key set does not contain (a rotation the client
        // has not caught up with). The signature stays key 1's: the refusal must come from
        // the kid lookup, not from the signature.
        let keys = IdpKeys::get();
        let header = json!({"alg": "RS256", "typ": "JWT", "kid": "retired-key-9"});
        let (_, rest) = signed.split_once('.').expect("header.payload.sig");
        let _ = keys;
        format!(
            "{}.{rest}",
            URL_SAFE_NO_PAD.encode(header.to_string().as_bytes())
        )
    }

    /// Stores an authorization code the way `/authorize` would.
    pub fn grant_code(&self, code: &str, challenge: &str, nonce: &str, redirect_uri: &str) {
        self.codes.lock().unwrap_or_else(|p| p.into_inner()).insert(
            code.to_string(),
            CodeGrant {
                challenge: challenge.to_string(),
                nonce: nonce.to_string(),
                redirect_uri: redirect_uri.to_string(),
            },
        );
    }
}

/// The token request, exactly the form `google.rs` posts. `grant_type` arrives and only the
/// authorization-code grant exists here, so it is accepted and unread (the field is prefixed
/// for serde to skip unknown-form errors while keeping the shape documented).
#[derive(Deserialize)]
struct TokenBody {
    code: String,
    code_verifier: String,
    redirect_uri: String,
    #[serde(rename = "grant_type")]
    _grant_type: String,
}

/// The JWKS document for the served keys (also read directly by the memory Idp path).
pub(crate) fn jwks_document() -> serde_json::Value {
    jwks_doc()
}

fn jwks_doc() -> serde_json::Value {
    let keys = IdpKeys::get();
    let one = |k: &IdpKey| {
        let e = k.public.e().to_bytes_be();
        let n = k.public.n().to_bytes_be();
        json!({
            "kty": "RSA", "alg": "RS256", "use": "sig", "kid": k.kid,
            "n": URL_SAFE_NO_PAD.encode(n),
            "e": URL_SAFE_NO_PAD.encode(e),
        })
    };
    json!({"keys": [one(&keys.0[0]), one(&keys.0[1])]})
}

/// `GET /__test-idp/jwks`.
async fn jwks(State((idp, _client_id)): State<(TestIdp, String)>) -> Response {
    idp.jwks_fetches.fetch_add(1, Ordering::SeqCst);
    let cc = idp
        .cache_control
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    let mut resp = Json(jwks_doc()).into_response();
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&cc).unwrap_or(HeaderValue::from_static("public, max-age=300")),
    );
    resp
}

/// `POST /__test-idp/token`: checks PKCE, spends the code, answers an id token. The body is
/// the form `google.rs` posts (`client_id`, `client_secret`, `grant_type` accepted, unused).
async fn token(
    State((idp, client_id)): State<(TestIdp, String)>,
    axum::Form(body): axum::Form<TokenBody>,
) -> Response {
    let mut codes = idp.codes.lock().unwrap_or_else(|p| p.into_inner());
    match codes.remove(&body.code) {
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response(),
        Some(grant) => {
            let digest = URL_SAFE_NO_PAD.encode(Sha256::digest(body.code_verifier.as_bytes()));
            *idp.last_verifier.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(body.code_verifier.clone());
            if digest != grant.challenge || body.redirect_uri != grant.redirect_uri {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid_grant"})),
                )
                    .into_response();
            }
            let claims = Claims {
                aud: Some(client_id.clone()),
                nonce: Some(Some(grant.nonce.clone())),
                ..Claims::for_sub("g-0")
            };
            let token = idp.id_token(&claims, Some(&grant.nonce), 0);
            Json(json!({"id_token": token, "access_token": "at", "token_type": "Bearer"}))
                .into_response()
        }
    }
}

/// `GET /__test-idp/authorize?...`: grants a code for the challenge and nonce in the query.
#[derive(Deserialize)]
struct AuthorizeQuery {
    code_challenge: String,
    nonce: String,
    redirect_uri: String,
    /// Echoed back with the code (the real endpoint does; our `start` reads its own cookie,
    /// but a driver that wants the pair has it).
    #[serde(default)]
    state: String,
}

async fn authorize(
    State((idp, _client_id)): State<(TestIdp, String)>,
    axum::extract::Query(q): axum::extract::Query<AuthorizeQuery>,
) -> Response {
    let code = URL_SAFE_NO_PAD.encode({
        let mut b = [0u8; 24];
        getrandom::getrandom(&mut b).expect("os random");
        b
    });
    idp.grant_code(&code, &q.code_challenge, &q.nonce, &q.redirect_uri);
    // The code rides a fragment-free query so the driver of the flow (a test, or the 03-03
    // browser stub) can complete it: a real Idp would redirect into the app with `code=`.
    (
        StatusCode::FOUND,
        [(
            "Location",
            format!("{}?code={code}&state={}", q.redirect_uri, q.state),
        )],
    )
        .into_response()
}

/// `POST /__test-idp/mint`: mints a token with the posted claims (One Tap stub).
/// `aud` defaults to the configured client id, like a real Google token would.
async fn mint(
    State((idp, client_id)): State<(TestIdp, String)>,
    Json(body): Json<MintBody>,
) -> Response {
    // Only `sub` is required; everything else defaults to a Google-shaped profile.
    let mut claims = Claims::for_sub(&body.sub).aud(&client_id);
    if let Some(email) = body.email {
        claims = claims.email(&email, body.email_verified.unwrap_or(true));
    }
    if let Some(nonce) = body.nonce {
        claims = claims.nonce(Some(&nonce));
    }
    if let Some(exp) = body.exp {
        claims = claims.exp(Some(Some(exp)));
    }
    Json(json!({"credential": idp.id_token(&claims, None, 0)})).into_response()
}

/// The mint request: `sub` plus any claim override the caller wants.
#[derive(Deserialize)]
struct MintBody {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    exp: Option<i64>,
}

/// The `/__test-idp` router. Mounted only in hosted mode with the feature. It carries its own
/// `(TestIdp, client_id)` state, so it nests into the app as a service (`nest_service`) with
/// no host-state coupling: `client_id` is the configured `GOOGLE_CLIENT_ID` (public; minted
/// credentials must carry it as `aud` to pass verification) and the Idp is the mounted one.
pub fn router(idp: TestIdp, client_id: String) -> Router<()> {
    Router::new()
        // Nested under `/__test-idp` by the host: the inner paths are relative.
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .route("/authorize", get(authorize))
        .route("/mint", post(mint))
        .with_state((idp, client_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_has_three_dot_parts_and_a_kid() {
        let idp = TestIdp::new();
        let t = idp.id_token(&Claims::for_sub("g-1"), Some("n1"), 0);
        let parts: Vec<&str> = t.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header["kid"], "test-key-0");
        assert_eq!(header["alg"], "RS256");
    }

    #[test]
    fn two_keys_exist_and_sign_differently() {
        let idp = TestIdp::new();
        assert_ne!(idp.sign(b"x", 0), idp.sign(b"x", 1));
        assert_eq!(
            idp.kids(),
            ["test-key-0".to_string(), "test-key-1".to_string()]
        );
    }

    #[test]
    fn the_clock_moves() {
        let c = ManualClock::new(1_700_000_000);
        assert_eq!(c.unix(), 1_700_000_000);
        c.advance(chrono::Duration::hours(25));
        assert_eq!(c.unix(), 1_700_090_000);
        assert_eq!(c.now().timestamp(), 1_700_090_000);
    }

    #[test]
    fn claims_can_drop_nonce_and_exp() {
        let idp = TestIdp::new();
        let t = idp.id_token(&Claims::for_sub("g-2").nonce(None).exp(Some(None)), None, 0);
        let parts: Vec<&str> = t.split('.').collect();
        let payload: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert!(payload.get("nonce").is_none());
        assert!(payload.get("exp").is_none());
    }
}
