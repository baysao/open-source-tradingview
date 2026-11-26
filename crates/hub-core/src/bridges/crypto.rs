//! Bridge token sealing (07-ARCHITECTURE decision 3): XChaCha20-Poly1305 under
//! `VTV_SECRET_KEY`, stored as `nonce || ciphertext` in `mt5_bridges.token_enc`. The key is
//! `HMAC-SHA256(secret, "mt5-bridge-token")` (the R21 key-separation pattern; the cookie-MAC key
//! is not reused). A random 24-byte nonce per record.

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The 24-byte nonce XChaCha20-Poly1305 wants.
const NONCE_LEN: usize = 24;

/// The bridge-token key: `HMAC-SHA256(secret_bytes, "mt5-bridge-token")`, 32 bytes.
pub fn bridge_key(secret: &[u8]) -> [u8; 32] {
    let mut mac =
        <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"mt5-bridge-token");
    mac.finalize().into_bytes().into()
}

/// Seals `token`: fresh random nonce, XChaCha20-Poly1305, `nonce || ciphertext`.
pub fn seal(key: &[u8; 32], token: &str) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).expect("os rng");
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), token.as_bytes())
        .expect("XChaCha20-Poly1305 seal cannot fail for an in-memory key");
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    out
}

/// Why [`open`] refused a blob. Deliberately opaque: the caller answers 404/422 with a fixed
/// message, and nothing here can leak key or token material through `Debug`.
#[derive(Debug)]
pub struct CryptoError;

/// Opens a `nonce || ciphertext` blob back into the token.
pub fn open(key: &[u8; 32], blob: &[u8]) -> Result<String, CryptoError> {
    if blob.len() <= NONCE_LEN {
        return Err(CryptoError);
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(key));
    let pt = cipher
        .decrypt(XNonce::from_slice(nonce), ct)
        .map_err(|_| CryptoError)?;
    String::from_utf8(pt).map_err(|_| CryptoError)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        bridge_key(b"test-secret-key-bytes-for-07-01")
    }

    #[test]
    fn roundtrip_and_failures() {
        let k = key();
        let blob = seal(&k, "tok-abc-123");
        assert_ne!(blob[..NONCE_LEN].to_vec(), vec![0u8; NONCE_LEN]);
        assert_eq!(open(&k, &blob).unwrap(), "tok-abc-123");
        // A wrong key cannot open it.
        let other = bridge_key(b"a-different-secret");
        assert!(open(&other, &blob).is_err());
        // A tampered ciphertext byte cannot open it.
        let mut bad = blob.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(open(&k, &bad).is_err());
        // Too-short blobs (nonce only, empty) are refused, not panics.
        assert!(open(&k, &blob[..NONCE_LEN]).is_err());
        assert!(open(&k, &[]).is_err());
        // Nonces are per-record: two seals of the same token differ in their nonce.
        let again = seal(&k, "tok-abc-123");
        assert_ne!(blob[..NONCE_LEN], again[..NONCE_LEN]);
    }

    #[test]
    fn empty_token_roundtrips() {
        let k = key();
        let blob = seal(&k, "");
        assert_eq!(open(&k, &blob).unwrap(), "");
    }
}
