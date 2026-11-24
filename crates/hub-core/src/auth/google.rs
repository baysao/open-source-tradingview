//! The Google client (private): JWKS fetch-and-cache plus the token exchange. `Idp::Http` is
//! reqwest against the configured endpoints; `Idp::Memory` (feature `test-idp`) calls the
//! [`TestIdp`](crate::auth::test_idp::TestIdp) directly, so a test can drive every flow without
//! a socket (L-016) while the HTTP path gets its own suite over a loopback one.
//!
//! The JWKS cache lives behind one std lock that is never held across an `.await`: a request
//! copies the decision inputs out, fetches without the lock, and stores under it again.

use std::sync::Mutex;

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use reqwest::Client as HttpClient;

use crate::auth::rules::{jwks_needs_fetch, jwks_ttl, KeySet};
#[cfg(feature = "test-idp")]
use crate::auth::test_idp::{Claims, TestIdp};

/// What a token exchange answers: the id token to verify.
pub(crate) struct TokenResponse {
    pub(crate) id_token: String,
}

/// One fetch attempt against the JWKS endpoint.
pub(crate) enum JwksFetch {
    Ok { keys: KeySet, ttl: Duration },
    Err,
}

pub(crate) enum Idp {
    Http {
        http: HttpClient,
        auth_url: String,
        token_url: String,
        jwks_url: String,
        client_id: String,
        client_secret: String,
        redirect_uri: String,
    },
    #[cfg(feature = "test-idp")]
    Memory(TestIdp),
}

impl Idp {
    /// GET the JWKS document and decode it into a [`KeySet`].
    pub(crate) async fn fetch_jwks(&self) -> JwksFetch {
        match self {
            Idp::Http { http, jwks_url, .. } => {
                let resp = match http.get(jwks_url).send().await {
                    Ok(r) => r,
                    Err(_) => return JwksFetch::Err,
                };
                let ttl = jwks_ttl(
                    resp.headers()
                        .get(reqwest::header::CACHE_CONTROL)
                        .and_then(|v| v.to_str().ok()),
                );
                let doc: serde_json::Value = match resp.json().await {
                    Ok(d) => d,
                    Err(_) => return JwksFetch::Err,
                };
                match decode_jwks(&doc) {
                    Some(keys) => JwksFetch::Ok { keys, ttl },
                    None => JwksFetch::Err,
                }
            }
            #[cfg(feature = "test-idp")]
            Idp::Memory(_) => JwksFetch::Err, // never fetched: see jwks_keys
        }
    }

    /// POST the authorization code for the id token. `Err(())` is any Idp refusal or outage;
    /// the caller turns it into `Refusal::IdpRefusedCode` or `AuthError::Unavailable`.
    pub(crate) async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<TokenResponse, ()> {
        match self {
            Idp::Http {
                http,
                token_url,
                client_id,
                client_secret,
                redirect_uri,
                ..
            } => {
                let params = [
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("code_verifier", verifier),
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("redirect_uri", redirect_uri.as_str()),
                ];
                let resp = http
                    .post(token_url)
                    .form(&params)
                    .send()
                    .await
                    .map_err(|_| ())?;
                if !resp.status().is_success() {
                    return Err(());
                }
                let body: serde_json::Value = resp.json().await.map_err(|_| ())?;
                let id_token = body
                    .get("id_token")
                    .and_then(|v| v.as_str())
                    .ok_or(())?
                    .to_string();
                Ok(TokenResponse { id_token })
            }
            #[cfg(feature = "test-idp")]
            Idp::Memory(_) => Err(()), // memory Idp mints directly; no code exchange
        }
    }
}

/// JWKS document to a [`KeySet`]: RSA signing keys only, `kid` required.
pub(crate) fn decode_jwks(doc: &serde_json::Value) -> Option<KeySet> {
    let keys = doc.get("keys")?.as_array()?;
    let mut out = KeySet::new();
    for k in keys {
        let kid = k.get("kid")?.as_str()?.to_string();
        let n = k.get("n")?.as_str()?;
        let e = k.get("e")?.as_str()?;
        let dec = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let n = dec.decode(n).ok()?;
        let e = dec.decode(e).ok()?;
        let jwk = jsonwebtoken::jwk::Jwk {
            common: jsonwebtoken::jwk::CommonParameters {
                public_key_use: None,
                key_operations: None,
                key_algorithm: Some(jsonwebtoken::jwk::KeyAlgorithm::RS256),
                key_id: Some(kid.clone()),
                x509_url: None,
                x509_chain: None,
                x509_sha1_fingerprint: None,
                x509_sha256_fingerprint: None,
            },
            algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(
                jsonwebtoken::jwk::RSAKeyParameters {
                    key_type: jsonwebtoken::jwk::RSAKeyType::RSA,
                    n: dec.encode(n),
                    e: dec.encode(e),
                },
            ),
        };
        let key = jsonwebtoken::DecodingKey::from_jwk(&jwk).ok()?;
        out.insert(kid, key);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The JWKS cache state. `fresh_until`/`last_fetch` from the last attempt (ok or failed);
/// the decision to fetch is [`jwks_needs_fetch`], pure and table-tested.
pub(crate) struct JwksCache {
    state: Mutex<JwksState>,
}

struct JwksState {
    keys: KeySet,
    fresh_until: Option<DateTime<Utc>>,
    last_fetch: Option<DateTime<Utc>>,
}

impl JwksCache {
    pub(crate) fn new() -> JwksCache {
        JwksCache {
            state: Mutex::new(JwksState {
                keys: KeySet::new(),
                fresh_until: None,
                last_fetch: None,
            }),
        }
    }

    /// The kid lookup under the lock (never across an await).
    pub(crate) fn kid_known(&self, kid: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys
            .get(kid)
            .is_some()
    }

    /// Copy of the decision inputs (drop the lock before any network).
    pub(crate) fn plan(&self, now: DateTime<Utc>, kid_known: bool) -> Option<bool> {
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        Some(jwks_needs_fetch(
            s.fresh_until,
            s.last_fetch,
            kid_known,
            now,
        ))
    }

    /// Store the result of a fetch attempt (ok or failed).
    pub(crate) fn store(&self, keys: KeySet, ttl: Duration, now: DateTime<Utc>) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.keys = keys;
        s.fresh_until = Some(now + ttl);
        s.last_fetch = Some(now);
    }

    pub(crate) fn store_failed(&self, now: DateTime<Utc>) {
        let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.last_fetch = Some(now);
    }

    /// A clone of the keys under the lock.
    pub(crate) fn keys(&self) -> KeySet {
        let s = self.state.lock().unwrap_or_else(|p| p.into_inner());
        s.keys.clone()
    }

    #[cfg(feature = "test-idp")]
    pub(crate) fn install(&self, keys: KeySet, ttl: Duration, now: DateTime<Utc>) {
        self.store(keys, ttl, now);
    }
}

/// A snapshot of the memory Idp's keys, installed once at build time (the memory path never
/// fetches over the network, so the cache is pre-filled and always fresh).
#[cfg(feature = "test-idp")]
pub(crate) fn memory_keys(idp: &TestIdp) -> KeySet {
    let mut set = KeySet::new();
    let doc = memory_jwks_doc(idp);
    if let Some(keys) = decode_jwks(&doc) {
        set = keys;
    }
    set
}

#[cfg(feature = "test-idp")]
fn memory_jwks_doc(idp: &TestIdp) -> serde_json::Value {
    // The TestIdp's own router serves this document; rebuild it from the same key material by
    // asking the router's handler through a direct construction here (keys are global).
    let _ = idp;
    crate::auth::test_idp::jwks_document()
}

/// The claims a memory Idp mints (One Tap and the token endpoint both end here).
#[cfg(feature = "test-idp")]
pub(crate) fn memory_id_token(
    idp: &TestIdp,
    claims: &Claims,
    nonce: Option<&str>,
    kid: usize,
) -> String {
    idp.id_token(claims, nonce, kid)
}
