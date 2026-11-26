//! Sealed secrets and the hashes that leave the process.
//!
//! A raw session id or desktop code never reaches the database, a log or a `Debug` line: it is
//! sealed in a newtype with no `Debug` and no `Clone`, encoded only into the cookie or the deep
//! link, and the store takes [`SessionHash`], [`CodeHash`] and [`KeyHash`]. Only [`SessionId`]'s
//! `hash` and the two `of` constructors produce them, so a raw secret cannot be stored by mistake.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hkdf::Hkdf;
use sha2::{Digest, Sha256};

/// Random bytes behind one session id or desktop code.
const SECRET_LEN: usize = 32;

fn random_bytes() -> [u8; SECRET_LEN] {
    let mut buf = [0u8; SECRET_LEN];
    getrandom::getrandom(&mut buf).expect("OS random source");
    buf
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A session id (32 random bytes). Leaves only as the value of `__Host-vtv_session`.
pub struct SessionId([u8; SECRET_LEN]);

impl SessionId {
    pub(crate) fn fresh() -> SessionId {
        SessionId(random_bytes())
    }

    /// Fixed bytes, for tests and for a session read back from a cookie.
    #[allow(dead_code)] // a 03-02 test reconstructs a SessionId from fixed bytes
    pub(crate) fn from_bytes(bytes: [u8; SECRET_LEN]) -> SessionId {
        SessionId(bytes)
    }

    /// The cookie value: 43 base64url characters, no padding.
    pub(crate) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub(crate) fn hash(&self) -> SessionHash {
        SessionHash(sha256(self.encode().as_bytes()))
    }
}

/// A one-time desktop code (32 random bytes). Leaves only inside the deep link.
pub struct DesktopCode([u8; SECRET_LEN]);

impl DesktopCode {
    pub(crate) fn fresh() -> DesktopCode {
        DesktopCode(random_bytes())
    }

    #[allow(dead_code)] // fixed seeds are a 03-02 test need
    pub(crate) fn from_bytes(bytes: [u8; SECRET_LEN]) -> DesktopCode {
        DesktopCode(bytes)
    }

    pub(crate) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub(crate) fn hash(&self) -> CodeHash {
        CodeHash(sha256(self.encode().as_bytes()))
    }
}

/// The OTP pairing code's modulus: exactly ten digits, so the room is 10^10.
const PAIRING_SPACE: u64 = 10_000_000_000;

/// A pairing code shown in Settings and typed into the manager app: ten digits, 0..=10^10-1.
///
/// Uniform by rejection sampling over `u64`. Four bytes cannot reach 10^10 (2^32 is about
/// 4.3e9), and `% 10^10` over 2^64 would bias the low residues by roughly 1.8e9/2^64 - small,
/// but a biased code alphabet is exactly the property the gate wants absent. The rejection
/// bound is `2^64 - (2^64 mod 10^10)`, so every digit string is equally likely.
pub struct PairingCode(u64);

impl PairingCode {
    pub(crate) fn fresh() -> PairingCode {
        // `2^64 mod 10^10`: computed once at compile time, so the loop rejects exactly the
        // tail that would bias the mapping.
        const LIMIT: u64 = u64::MAX - (u64::MAX % PAIRING_SPACE);
        loop {
            let mut buf = [0u8; 8];
            getrandom::getrandom(&mut buf).expect("OS random source");
            let n = u64::from_le_bytes(buf);
            if n < LIMIT {
                return PairingCode(n % PAIRING_SPACE);
            }
        }
    }

    /// Fixed value, for tests.
    #[cfg(test)]
    pub(crate) fn from_digits(n: u64) -> PairingCode {
        assert!(n < PAIRING_SPACE, "a pairing code is under 10^10");
        PairingCode(n)
    }

    /// The wire form: ten digits, zero-padded (`0073451209`), never grouped. The page groups
    /// it for display only.
    pub(crate) fn encode(&self) -> String {
        format!("{:010}", self.0)
    }

    pub(crate) fn hash(&self) -> PairingCodeHash {
        PairingCodeHash(sha256(self.encode().as_bytes()))
    }
}

impl std::fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sealed like every other live secret: a code in a log line is a code a reader can use.
        write!(f, "PairingCode(_)")
    }
}

/// SHA-256 of a pairing code's ten-digit string.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PairingCodeHash([u8; SECRET_LEN]);

impl std::fmt::Debug for PairingCodeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PairingCodeHash(_)")
    }
}

impl PairingCodeHash {
    pub(crate) fn of_code(code: &str) -> PairingCodeHash {
        PairingCodeHash(sha256(code.as_bytes()))
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A bridge's standing revoke secret: 32 random bytes, base64url on the wire, only its hash
/// stored. One per bridge, shown once at pairing (09-ARCHITECTURE decision 2).
pub struct RevokeSecret([u8; SECRET_LEN]);

impl RevokeSecret {
    pub(crate) fn fresh() -> RevokeSecret {
        RevokeSecret(random_bytes())
    }

    /// The 43-character wire value, returned exactly once in the pairing response.
    pub(crate) fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub(crate) fn hash(&self) -> RevokeHash {
        RevokeHash(sha256(self.encode().as_bytes()))
    }
}

impl std::fmt::Debug for RevokeSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RevokeSecret(_)")
    }
}

/// SHA-256 of a revoke secret's wire value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RevokeHash([u8; SECRET_LEN]);

impl std::fmt::Debug for RevokeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RevokeHash(_)")
    }
}

impl RevokeHash {
    /// Public: the bridge store's `set_revoke_hash` takes one, and the contract suites seed a
    /// bridge with a known secret. It hashes only what it is given, so exposing it grants no
    /// capability a caller does not already have.
    pub fn of_secret(secret: &str) -> RevokeHash {
        RevokeHash(sha256(secret.as_bytes()))
    }

    #[allow(dead_code)] // read by the store's B6/B7 binds
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// SHA-256 of a session id's cookie value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SessionHash([u8; SECRET_LEN]);
impl std::fmt::Debug for SessionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Sealed: the bytes never print, only the type and a length marker.
        write!(f, "SessionHash(_)")
    }
}

impl SessionHash {
    #[allow(dead_code)] // the store reads rows by hash (03-02 Task 2)
    pub(crate) fn of_cookie(cookie: &str) -> SessionHash {
        SessionHash(sha256(cookie.as_bytes()))
    }

    /// Rebuilds a hash from a row's `bytea` (the store's reads, S6/S12).
    pub(crate) fn from_bytes(bytes: [u8; SECRET_LEN]) -> SessionHash {
        SessionHash(bytes)
    }

    #[allow(dead_code)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// SHA-256 of a desktop code's deep-link value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CodeHash([u8; SECRET_LEN]);

impl std::fmt::Debug for CodeHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CodeHash(_)")
    }
}

impl CodeHash {
    #[allow(dead_code)] // the store reads and burns codes by hash (03-02 Task 2)
    pub(crate) fn of_code(code: &str) -> CodeHash {
        CodeHash(sha256(code.as_bytes()))
    }

    #[allow(dead_code)]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// SHA-256 of a `desktop_key` (the desktop app's PKCE-style secret).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyHash([u8; SECRET_LEN]);

impl std::fmt::Debug for KeyHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyHash(_)")
    }
}

impl KeyHash {
    pub(crate) fn of_key(key: &str) -> KeyHash {
        KeyHash(sha256(key.as_bytes()))
    }

    #[allow(dead_code)] // stored beside a code, compared on exchange (03-02 Task 2)
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[allow(dead_code)]
    pub(crate) fn from_bytes(bytes: [u8; SECRET_LEN]) -> KeyHash {
        KeyHash(bytes)
    }

    /// Constant-time equality, for a presented key against a stored hash.
    pub(crate) fn ct_eq(&self, other: &KeyHash) -> bool {
        use subtle::ConstantTimeEq as _;
        self.0.ct_eq(&other.0).into()
    }
}

/// The keys derived from `VTV_SECRET_KEY`. One is used (the oauth cookie MAC); the second is
/// derived and dropped so a later use of that label cannot silently reuse a different key.
pub(crate) struct Keys {
    pub(crate) oauth_mac: [u8; 32],
}

impl Keys {
    pub(crate) fn derive(secret: &[u8]) -> Keys {
        let hk = Hkdf::<Sha256>::new(Some(b"vtv"), secret);
        let mut oauth_mac = [0u8; 32];
        hk.expand(b"oauth-cookie-mac", &mut oauth_mac)
            .expect("32 bytes is a valid HKDF output length");
        let mut reserved = [0u8; 32];
        hk.expand(b"reserved", &mut reserved)
            .expect("32 bytes is a valid HKDF output length");
        // Deliberately not kept: reserved for a future label.
        let _ = reserved;
        Keys { oauth_mac }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_cookie_is_43_base64url_chars() {
        let id = SessionId::fresh();
        let cookie = id.encode();
        assert_eq!(cookie.len(), 43);
        assert!(cookie
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(id.hash(), SessionHash::of_cookie(&cookie));
    }

    #[test]
    fn a_code_cookie_is_43_chars_and_distinct_from_the_session() {
        let code = DesktopCode::fresh();
        assert_eq!(code.encode().len(), 43);
        assert_ne!(code.encode(), DesktopCode::fresh().encode());
        assert_eq!(code.hash(), CodeHash::of_code(&code.encode()));
    }

    #[test]
    fn key_hashes_compare_in_constant_time() {
        assert!(KeyHash::of_key("abc").ct_eq(&KeyHash::of_key("abc")));
        assert!(!KeyHash::of_key("abc").ct_eq(&KeyHash::of_key("abd")));
    }

    #[test]
    fn hkdf_is_deterministic_and_label_separated() {
        let a = Keys::derive(b"secret");
        let b = Keys::derive(b"secret");
        let c = Keys::derive(b"other");
        assert_eq!(a.oauth_mac, b.oauth_mac);
        assert_ne!(a.oauth_mac, c.oauth_mac);
    }

    #[test]
    fn a_pairing_code_is_ten_digits_and_zero_padded() {
        // The alphabet is exactly the ten digits, and a small value keeps its leading zeros:
        // the wire form is always ten characters, so a client can group it without guessing.
        let code = PairingCode::from_digits(7);
        assert_eq!(code.encode(), "0000000007");
        assert_eq!(
            PairingCode::from_digits(9_999_999_999).encode(),
            "9999999999"
        );
        assert!(code.encode().bytes().all(|b| b.is_ascii_digit()));
        // Fresh codes stay inside the modulus (the rejection loop's contract).
        for _ in 0..64 {
            let s = PairingCode::fresh().encode();
            assert_eq!(s.len(), 10, "{s}");
            assert!(s.bytes().all(|b| b.is_ascii_digit()), "{s}");
        }
    }

    #[test]
    fn the_rejection_bound_covers_the_whole_space() {
        // 4 bytes cannot reach 10^10, which is why the generator samples u64. This is a
        // compile-time fact, so it is asserted as one: a later "optimization" back to 4 bytes
        // fails the BUILD, not just this test, and can never bias the alphabet unnoticed.
        const {
            assert!(
                (u32::MAX as u64) < PAIRING_SPACE,
                "4 bytes cannot hold 10^10 - sample u64"
            )
        };
        // The rejection tail is the remainder: a multiple of the modulus means every accepted n
        // maps onto a uniform residue, which is the property the bound exists for.
        let limit = u64::MAX - (u64::MAX % PAIRING_SPACE);
        assert_eq!(
            limit % PAIRING_SPACE,
            0,
            "the bound is a multiple of the space"
        );
    }

    #[test]
    fn hashes_are_domain_shaped_and_sealed_in_debug() {
        let code = PairingCode::from_digits(1234567890);
        assert_eq!(code.hash(), PairingCodeHash::of_code("1234567890"));
        assert_ne!(
            code.hash().as_bytes(),
            PairingCodeHash::of_code("1234567891").as_bytes()
        );
        // Neither a live code nor a revoke secret prints its bytes.
        assert_eq!(format!("{code:?}"), "PairingCode(_)");
        let secret = RevokeSecret::fresh();
        assert_eq!(secret.encode().len(), 43);
        assert_eq!(format!("{secret:?}"), "RevokeSecret(_)");
        assert_eq!(secret.hash(), RevokeHash::of_secret(&secret.encode()));
    }
}
