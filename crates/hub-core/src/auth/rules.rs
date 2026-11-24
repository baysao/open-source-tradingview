//! Pure auth decisions. Every clock reading and every byte arrives as an argument, so each edge is
//! a table test with no harness and no I/O.

use crate::auth::ids::{KeyHash, Keys};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The public keys of the identity provider, by `kid`.
#[derive(Default, Clone)]
pub(crate) struct KeySet {
    keys: HashMap<String, DecodingKey>,
}

impl KeySet {
    pub(crate) fn new() -> KeySet {
        KeySet::default()
    }

    pub(crate) fn insert(&mut self, kid: impl Into<String>, key: DecodingKey) {
        self.keys.insert(kid.into(), key);
    }

    pub(crate) fn get(&self, kid: &str) -> Option<&DecodingKey> {
        self.keys.get(kid)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// What the caller expects of a token: our audience, Google's issuers, and the nonce we sent.
pub(crate) struct Expect<'a> {
    pub client_id: &'a str,
    pub issuers: &'a [String],
    pub nonce: Option<&'a str>,
}

/// A verified Google identity. No public constructor: the only path to a user row is
/// [`verify_id_token`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedIdentity {
    pub(crate) subject: String,
    pub(crate) email: String,
    pub(crate) email_verified: bool,
    pub(crate) name: Option<String>,
    pub(crate) picture: Option<String>,
}

/// Why an auth step was refused. Logged as the variant name only, never with the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Malformed,
    AlgNotRs256,
    UnknownKid,
    BadSignature,
    WrongAudience,
    WrongIssuer,
    Expired,
    NonceMismatch,
    MissingClaim(&'static str),
    TicketMissing,
    TicketBadMac,
    TicketExpired,
    StateMismatch,
    IdpRefusedCode,
    CodeUnknown,
    CodeReused,
    CodeExpired,
    DesktopKeyMismatch,
}

#[derive(Deserialize)]
struct Claims {
    aud: Option<Value>,
    iss: Option<String>,
    exp: Option<i64>,
    nonce: Option<String>,
    sub: Option<String>,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    picture: Option<String>,
}

/// alg must be RS256 before any key lookup; the signature is checked with jsonwebtoken's own time
/// checks off; then aud, iss, `exp > now`, nonce when expected, and sub plus email present.
pub(crate) fn verify_id_token(
    token: &str,
    keys: &KeySet,
    x: &Expect<'_>,
    now: DateTime<Utc>,
) -> Result<VerifiedIdentity, Refusal> {
    let header = decode_header(token).map_err(|_| Refusal::Malformed)?;
    if header.alg != Algorithm::RS256 {
        return Err(Refusal::AlgNotRs256);
    }
    let kid = header.kid.ok_or(Refusal::UnknownKid)?;
    let key = keys.get(&kid).ok_or(Refusal::UnknownKid)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.validate_nbf = false;
    validation.required_spec_claims.clear();

    let claims = decode::<Claims>(token, key, &validation)
        .map_err(|e| match e.kind() {
            jsonwebtoken::errors::ErrorKind::InvalidSignature => Refusal::BadSignature,
            _ => Refusal::Malformed,
        })?
        .claims;

    let audience_ok = match claims.aud.as_ref() {
        Some(Value::String(aud)) => aud == x.client_id,
        Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(x.client_id)),
        _ => false,
    };
    if !audience_ok {
        return Err(Refusal::WrongAudience);
    }

    let issuer = claims.iss.as_deref().ok_or(Refusal::WrongIssuer)?;
    if !x.issuers.iter().any(|i| i == issuer) {
        return Err(Refusal::WrongIssuer);
    }

    let exp = claims.exp.ok_or(Refusal::MissingClaim("exp"))?;
    if exp <= now.timestamp() {
        return Err(Refusal::Expired);
    }

    if let Some(expected) = x.nonce {
        if claims.nonce.as_deref() != Some(expected) {
            return Err(Refusal::NonceMismatch);
        }
    }

    let subject = claims.sub.ok_or(Refusal::MissingClaim("sub"))?;
    let email = claims.email.ok_or(Refusal::MissingClaim("email"))?;

    Ok(VerifiedIdentity {
        subject,
        email,
        email_verified: claims.email_verified.unwrap_or(false),
        name: claims.name,
        picture: claims.picture,
    })
}

/// `max-age` from Cache-Control, clamped to 300..=86400 seconds; absent or unparsable is 300.
pub(crate) fn jwks_ttl(cache_control: Option<&str>) -> Duration {
    let secs = cache_control
        .and_then(|cc| {
            cc.split(',').find_map(|part| {
                let (name, value) = part.trim().split_once('=')?;
                if name.trim().eq_ignore_ascii_case("max-age") {
                    value.trim().parse::<u64>().ok()
                } else {
                    None
                }
            })
        })
        .unwrap_or(300)
        .clamp(300, 86_400);
    Duration::seconds(secs as i64)
}

/// Fetch when the set is empty or stale; an unknown kid fetches only once the last attempt (ok or
/// failed) is at least 60 s old.
pub(crate) fn jwks_needs_fetch(
    fresh_until: Option<DateTime<Utc>>,
    last_fetch: Option<DateTime<Utc>>,
    kid_known: bool,
    now: DateTime<Utc>,
) -> bool {
    match fresh_until {
        None => return true,
        Some(fresh) if now >= fresh => return true,
        Some(_) => {}
    }
    if kid_known {
        return false;
    }
    match last_fetch {
        None => true,
        Some(last) => now.signed_duration_since(last).num_seconds() >= 60,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionCheck {
    Expired,
    Valid,
    Slide {
        expires_at: DateTime<Utc>,
        last_seen_at: DateTime<Utc>,
    },
}

/// A session lives 30 days; any request after 24 h without activity slides both ends.
pub(crate) fn session_check(
    expires_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> SessionCheck {
    if now >= expires_at {
        return SessionCheck::Expired;
    }
    if now < last_seen_at + Duration::hours(24) {
        return SessionCheck::Valid;
    }
    SessionCheck::Slide {
        expires_at: now + Duration::days(30),
        last_seen_at: now,
    }
}

/// Which client began the sign-in. A desktop key is 22..=128 base64url characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Client {
    Web,
    Desktop(KeyHash),
}

pub(crate) fn parse_client(client: Option<&str>, dk: Option<&str>) -> Result<Client, Refusal> {
    match client {
        None | Some("web") => Ok(Client::Web),
        Some("desktop") => {
            let dk = dk.ok_or(Refusal::Malformed)?;
            let shape_ok = (22..=128).contains(&dk.len())
                && dk
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if !shape_ok {
                return Err(Refusal::Malformed);
            }
            Ok(Client::Desktop(KeyHash::of_key(dk)))
        }
        Some(_) => Err(Refusal::Malformed),
    }
}

/// A presented desktop key against a stored hash: expiry first, then a constant-time compare.
pub(crate) fn decide_exchange(
    key_hash: &KeyHash,
    expires_at: DateTime<Utc>,
    presented: &str,
    now: DateTime<Utc>,
) -> Result<(), Refusal> {
    if now >= expires_at {
        return Err(Refusal::CodeExpired);
    }
    if !key_hash.ct_eq(&KeyHash::of_key(presented)) {
        return Err(Refusal::DesktopKeyMismatch);
    }
    Ok(())
}

pub(crate) fn is_admin(email: &str, email_verified: bool, admins: &[String]) -> bool {
    email_verified && admins.iter().any(|a| a.eq_ignore_ascii_case(email))
}

/// The sealed `__Host-vtv_oauth` value: what the browser sends back alongside `state` and `code`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OauthTicket {
    pub(crate) state: String,
    pub(crate) nonce: String,
    pub(crate) verifier: String,
    pub(crate) client: Client,
    pub(crate) dk_hash: Option<KeyHash>,
    pub(crate) exp: i64,
}

#[derive(Serialize, Deserialize)]
struct TicketWire {
    state: String,
    nonce: String,
    verifier: String,
    client: String,
    dk_hash: Option<String>,
    exp: i64,
}

type HmacSha256 = Hmac<Sha256>;

impl OauthTicket {
    /// `b64url(json) "." b64url(HMAC-SHA256(payload))`.
    pub(crate) fn seal(&self, keys: &Keys) -> String {
        let (client, dk_hash) = match &self.client {
            Client::Web => ("web".to_string(), None),
            Client::Desktop(_) => (
                "desktop".to_string(),
                self.dk_hash.map(|h| URL_SAFE_NO_PAD.encode(h.as_bytes())),
            ),
        };
        let wire = TicketWire {
            state: self.state.clone(),
            nonce: self.nonce.clone(),
            verifier: self.verifier.clone(),
            client,
            dk_hash,
            exp: self.exp,
        };
        let payload =
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wire).expect("a ticket always serializes"));
        let mut mac =
            HmacSha256::new_from_slice(&keys.oauth_mac).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        let sig = mac.finalize().into_bytes();
        format!("{payload}.{}", URL_SAFE_NO_PAD.encode(sig))
    }

    /// Verify the MAC, then the expiry. Any malformed shape is a missing ticket.
    pub(crate) fn open(raw: &str, keys: &Keys, now: i64) -> Result<OauthTicket, Refusal> {
        let (payload, sig) = raw.split_once('.').ok_or(Refusal::TicketMissing)?;
        let sig = URL_SAFE_NO_PAD
            .decode(sig)
            .map_err(|_| Refusal::TicketBadMac)?;
        let mut mac =
            HmacSha256::new_from_slice(&keys.oauth_mac).expect("HMAC accepts any key length");
        mac.update(payload.as_bytes());
        mac.verify_slice(&sig).map_err(|_| Refusal::TicketBadMac)?;

        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| Refusal::TicketBadMac)?;
        let wire: TicketWire = serde_json::from_slice(&bytes).map_err(|_| Refusal::TicketBadMac)?;

        let (client, dk_hash) = match wire.client.as_str() {
            "web" => (Client::Web, None),
            "desktop" => {
                let dk_hash = wire.dk_hash.ok_or(Refusal::TicketBadMac)?;
                let bytes = URL_SAFE_NO_PAD
                    .decode(dk_hash)
                    .map_err(|_| Refusal::TicketBadMac)?;
                let bytes: [u8; 32] = bytes.try_into().map_err(|_| Refusal::TicketBadMac)?;
                let hash = KeyHash::from_bytes(bytes);
                (Client::Desktop(hash), Some(hash))
            }
            _ => return Err(Refusal::TicketBadMac),
        };

        if now >= wire.exp {
            return Err(Refusal::TicketExpired);
        }

        Ok(OauthTicket {
            state: wire.state,
            nonce: wire.nonce,
            verifier: wire.verifier,
            client,
            dk_hash,
            exp: wire.exp,
        })
    }

    /// PKCE S256: `b64url(SHA-256(verifier))`.
    pub(crate) fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIENT: &str = "client-id";

    fn key_set() -> KeySet {
        // Real key material is unnecessary: the signature edge is covered by
        // `auth_connectivity_and_tokens` over a real socket. Here the checks are order and
        // refusal reasons, so a set with one decodable-format key entry is enough.
        let mut ks = KeySet::new();
        ks.insert(
            "k0",
            DecodingKey::from_base64_secret("c29tZVJhbmRvbUJ5dGVz").expect("base64"),
        );
        ks
    }

    fn expect<'a>(nonce: Option<&'a str>) -> Expect<'a> {
        static ISSUERS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let issuers = ISSUERS.get_or_init(|| {
            vec![
                "https://accounts.google.com".to_string(),
                "accounts.google.com".to_string(),
            ]
        });
        Expect {
            client_id: CLIENT,
            issuers,
            nonce,
        }
    }

    // ---------- jwks_ttl ----------

    #[test]
    fn jwks_ttl_reads_max_age_and_clamps() {
        assert_eq!(
            jwks_ttl(Some("public, max-age=600")),
            Duration::seconds(600)
        );
        assert_eq!(jwks_ttl(Some("max-age=1")), Duration::seconds(300)); // clamped up
        assert_eq!(jwks_ttl(Some("max-age=999999")), Duration::seconds(86_400));
        assert_eq!(jwks_ttl(Some("no-cache")), Duration::seconds(300)); // absent
        assert_eq!(jwks_ttl(Some("max-age=bananas")), Duration::seconds(300));
        assert_eq!(jwks_ttl(None), Duration::seconds(300));
    }

    // ---------- jwks_needs_fetch ----------

    #[test]
    fn jwks_fetch_is_scheduled_by_freshness_and_unknown_kid() {
        let now = Utc::now();
        let fresh = now + Duration::hours(1);
        let stale = now - Duration::hours(1);

        // Empty set fetches, always.
        assert!(jwks_needs_fetch(None, None, false, now));

        // Fresh and the kid is known: no fetch.
        assert!(!jwks_needs_fetch(Some(fresh), Some(now), true, now));

        // Fresh but unknown kid, last attempt recent: no fetch (60 s throttle).
        assert!(!jwks_needs_fetch(Some(fresh), Some(now), false, now));

        // Fresh but unknown kid, last attempt over 60 s old: fetch once more.
        assert!(jwks_needs_fetch(
            Some(fresh),
            Some(now - Duration::seconds(61)),
            false,
            now
        ));
        // Unknown kid with no recorded attempt: fetch.
        assert!(jwks_needs_fetch(Some(fresh), None, false, now));

        // Stale: fetch regardless of kid.
        assert!(jwks_needs_fetch(Some(stale), Some(now), true, now));
    }

    // ---------- session_check ----------

    #[test]
    fn sessions_expire_slide_and_stay_valid() {
        let now = Utc::now();
        // Past expiry: refused.
        assert_eq!(
            session_check(now - Duration::seconds(1), now, now),
            SessionCheck::Expired
        );
        // Expiry is inclusive: at exactly expires_at the session is over.
        assert_eq!(session_check(now, now, now), SessionCheck::Expired);
        // Active within 24 h: valid, nothing slides.
        assert_eq!(
            session_check(now + Duration::days(20), now - Duration::hours(3), now),
            SessionCheck::Valid
        );
        // Idle beyond 24 h: both ends slide to now + 30 d.
        let slid = session_check(now + Duration::days(20), now - Duration::hours(25), now);
        match slid {
            SessionCheck::Slide {
                expires_at,
                last_seen_at,
            } => {
                assert_eq!(expires_at, now + Duration::days(30));
                assert_eq!(last_seen_at, now);
            }
            other => panic!("expected Slide, got {other:?}"),
        }
    }

    // ---------- parse_client / decide_exchange ----------

    #[test]
    fn client_parsing_accepts_web_and_shaped_desktop_keys() {
        assert_eq!(parse_client(None, None), Ok(Client::Web));
        assert_eq!(parse_client(Some("web"), None), Ok(Client::Web));

        // 22 chars is the floor; the shape is base64url only.
        let k22 = "abcdefghij_klmnopqrstu"; // 22 chars
        assert_eq!(k22.len(), 22);
        assert!(matches!(
            parse_client(Some("desktop"), Some(k22)),
            Ok(Client::Desktop(_))
        ));

        // 21 chars: too short. 129 chars: too long. Bad alphabet: refused.
        assert_eq!(
            parse_client(Some("desktop"), Some("abcdefghij_klmnopqrst")),
            Err(Refusal::Malformed)
        );
        let long = "a".repeat(129);
        assert_eq!(
            parse_client(Some("desktop"), Some(long.as_str())),
            Err(Refusal::Malformed)
        );
        assert_eq!(
            parse_client(Some("desktop"), Some("has space inside_123456789")),
            Err(Refusal::Malformed)
        );
        assert_eq!(parse_client(Some("desktop"), None), Err(Refusal::Malformed));
        assert_eq!(parse_client(Some("app"), None), Err(Refusal::Malformed));
    }

    #[test]
    fn code_exchange_checks_expiry_before_the_key() {
        let stored = KeyHash::of_key("good-key-abcdefghijklmnop");
        let now = Utc::now();

        // Expired wins over a wrong key: nothing about the key is learned.
        assert_eq!(
            decide_exchange(&stored, now - Duration::seconds(1), "wrong", now),
            Err(Refusal::CodeExpired)
        );
        // In window, wrong key: mismatch.
        assert_eq!(
            decide_exchange(&stored, now + Duration::seconds(60), "wrong", now),
            Err(Refusal::DesktopKeyMismatch)
        );
        // In window, right key: ok.
        assert_eq!(
            decide_exchange(
                &stored,
                now + Duration::seconds(60),
                "good-key-abcdefghijklmnop",
                now
            ),
            Ok(())
        );
    }

    // ---------- is_admin ----------

    #[test]
    fn admin_needs_a_verified_email_and_is_case_insensitive() {
        let admins = vec!["Owner@Example.com".to_string()];
        assert!(is_admin("owner@example.com", true, &admins));
        assert!(!is_admin("owner@example.com", false, &admins));
        assert!(!is_admin("someone@example.com", true, &admins));
        assert!(admins.is_empty() || !is_admin("owner@example.com", true, &[]));
    }

    // ---------- OauthTicket ----------

    fn ticket(client: Client, dk_hash: Option<KeyHash>) -> OauthTicket {
        OauthTicket {
            state: "st-abc123".to_string(),
            nonce: "no-abc123".to_string(),
            verifier: "ver-abcdefghijklmnop".to_string(),
            client,
            dk_hash,
            exp: 1_700_000_100,
        }
    }

    #[test]
    fn the_ticket_seals_and_opens_round_trip() {
        let keys = Keys::derive(b"secret");
        let t = ticket(Client::Web, None);
        let sealed = t.seal(&keys);
        let opened = OauthTicket::open(&sealed, &keys, 1_700_000_000).expect("opens");
        assert_eq!(opened, t);
    }

    #[test]
    fn the_ticket_survives_a_desktop_round_trip_with_its_key_hash() {
        let keys = Keys::derive(b"secret");
        let dk = KeyHash::of_key("desktop-key-abcdefghijklmnop");
        let t = ticket(Client::Desktop(dk), Some(dk));
        let opened = OauthTicket::open(&t.seal(&keys), &keys, 1_700_000_000).expect("opens");
        assert_eq!(opened, t);
        assert!(opened.dk_hash.is_some());
    }

    #[test]
    fn a_ticket_with_a_tampered_payload_refuses_the_mac() {
        let keys = Keys::derive(b"secret");
        let other = Keys::derive(b"different");
        let sealed = ticket(Client::Web, None).seal(&keys);

        // Wrong MAC key (forged seal).
        assert_eq!(
            OauthTicket::open(&sealed, &other, 1_700_000_000),
            Err(Refusal::TicketBadMac)
        );

        // Flipped payload byte under the correct key.
        let (payload, sig) = sealed.split_once('.').unwrap();
        let mut bytes = URL_SAFE_NO_PAD.decode(payload).unwrap();
        bytes[0] ^= 0x01;
        let forged = format!("{}.{}", URL_SAFE_NO_PAD.encode(&bytes), sig);
        assert_eq!(
            OauthTicket::open(&forged, &keys, 1_700_000_000),
            Err(Refusal::TicketBadMac)
        );

        // Expired: MAC ok, clock refuses.
        assert_eq!(
            OauthTicket::open(&sealed, &keys, 1_700_000_100),
            Err(Refusal::TicketExpired)
        );

        // Shape errors degrade to TicketMissing, never a panic.
        assert_eq!(
            OauthTicket::open("no-dot", &keys, 1_700_000_000),
            Err(Refusal::TicketMissing)
        );
    }

    #[test]
    fn the_ticket_challenge_is_pkce_s256() {
        let t = ticket(Client::Web, None);
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(t.verifier.as_bytes()));
        assert_eq!(t.challenge(), expected);
        // 43 chars, no padding.
        assert_eq!(t.challenge().len(), 43);
    }

    // ---------- verify_id_token: order and refusal reasons ----------

    #[test]
    fn a_garbage_token_is_malformed() {
        let ks = key_set();
        let now = Utc::now();
        assert_eq!(
            verify_id_token("not-a-jwt", &ks, &expect(Some("n")), now),
            Err(Refusal::Malformed)
        );
    }

    #[test]
    fn an_hs256_token_is_refused_before_any_key_lookup() {
        let now = Utc::now();
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(br#"{"iss":"https://accounts.google.com"}"#);
        let hs256 = format!("{header}.{body}.c2ln");
        assert_eq!(
            verify_id_token(&hs256, &key_set(), &expect(Some("n")), now),
            Err(Refusal::AlgNotRs256)
        );
    }

    #[test]
    fn an_unknown_kid_refuses_without_touching_the_signature() {
        let now = Utc::now();
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT","kid":"nope"}"#);
        let body = URL_SAFE_NO_PAD.encode(br#"{}"#);
        let token = format!("{header}.{body}.c2ln");
        assert_eq!(
            verify_id_token(&token, &key_set(), &expect(Some("n")), now),
            Err(Refusal::UnknownKid)
        );
        // No kid at all: same refusal.
        let header_nokid = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let token_nokid = format!("{header_nokid}.{body}.c2ln");
        assert_eq!(
            verify_id_token(&token_nokid, &key_set(), &expect(Some("n")), now),
            Err(Refusal::UnknownKid)
        );
    }

    #[test]
    fn the_issuer_must_be_google_and_the_audience_ours() {
        // A structurally valid RS256-shaped token signed by nobody: signature and claim edges
        // are covered over a real socket in `auth_connectivity_and_tokens`. This test pins only
        // that a RS256 header with a known kid reaches the signature check and stops there.
        let now = Utc::now();
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT","kid":"k0"}"#);
        let body = URL_SAFE_NO_PAD.encode(br#"{}"#);
        let token = format!("{header}.{body}.c2ln");
        assert_eq!(
            verify_id_token(&token, &key_set(), &expect(Some("n")), now),
            Err(Refusal::Malformed),
            "an undecodable body stops at the signature edge"
        );
    }

    #[cfg(feature = "test-idp")]
    #[test]
    fn a_token_with_the_wrong_nonce_is_refused() {
        use crate::auth::test_idp::{Claims, TestIdp};
        let idp = TestIdp::new();
        let ks = crate::auth::google::memory_keys(&idp);
        let now = Utc::now();
        let token = idp.id_token(&Claims::for_sub("g-1").aud(CLIENT), Some("nonce-a"), 0);
        // The matching nonce passes every other check (aud, iss, exp, sub, email).
        assert!(verify_id_token(&token, &ks, &expect(Some("nonce-a")), now).is_ok());
        // A different nonce is refused, and only for that reason.
        assert_eq!(
            verify_id_token(&token, &ks, &expect(Some("nonce-b")), now),
            Err(Refusal::NonceMismatch)
        );
        // With no nonce expected, the claim is not consulted.
        assert!(verify_id_token(&token, &ks, &expect(None), now).is_ok());
    }

    #[test]
    fn refusal_reasons_order_the_checks() {
        // The Refusal enum is the audit trail; make sure no variant collides in Debug output.
        let reasons = [
            Refusal::Malformed,
            Refusal::AlgNotRs256,
            Refusal::UnknownKid,
            Refusal::BadSignature,
            Refusal::WrongAudience,
            Refusal::WrongIssuer,
            Refusal::Expired,
            Refusal::NonceMismatch,
            Refusal::MissingClaim("exp"),
            Refusal::TicketMissing,
            Refusal::TicketBadMac,
            Refusal::TicketExpired,
            Refusal::StateMismatch,
            Refusal::IdpRefusedCode,
            Refusal::CodeUnknown,
            Refusal::CodeReused,
            Refusal::CodeExpired,
            Refusal::DesktopKeyMismatch,
        ];
        for (i, a) in reasons.iter().enumerate() {
            for b in reasons.iter().skip(i + 1) {
                assert!(a != b, "distinct refusal reasons must stay distinct: {a:?}");
            }
        }
    }
}
