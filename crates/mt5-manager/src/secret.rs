//! Secrets that cannot be printed by accident.
//!
//! The revoke code is shown once in the UI and must never reach a log line (AC-3). A `String`
//! cannot promise that; a type whose `Debug` prints a length can, and the redaction then holds for
//! every `{secret:?}`, `dbg!` and panic message in the crate.
//!
//! There is deliberately no `Display`: printing a secret has to be asked for by name, through
//! [`Secret::expose`], which is greppable in review.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;

/// Bytes behind a bridge token or a revoke code. 32 gives 43 base64url chars.
const SECRET_BYTES: usize = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// 32 OS random bytes as base64url without padding. Used for the generated `BRIDGE_TOKEN`.
    pub fn mint() -> Self {
        let mut buf = [0u8; SECRET_BYTES];
        getrandom::getrandom(&mut buf).expect("the OS random source");
        Secret(URL_SAFE_NO_PAD.encode(buf))
    }

    /// The only way to read the value. Every call site is a deliberate one.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Redacted: a log line, a panic or a `{:?}` on a struct holding one prints a length, never value.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<secret: {} chars>", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_prints_as_a_length_never_its_value() {
        let s = Secret::new("super-secret-value".to_string());
        let printed = format!("{s:?}");
        assert_eq!(printed, "<secret: 18 chars>");
        assert!(!printed.contains("super"));
    }

    #[test]
    fn a_struct_holding_a_secret_does_not_leak_it_through_debug() {
        #[derive(Debug)]
        struct Holder {
            #[allow(dead_code)]
            token: Secret,
            label: String,
        }
        let h = Holder {
            token: Secret::new("abc123".to_string()),
            label: "host".to_string(),
        };
        let printed = format!("{h:?}");
        assert_eq!(h.label, "host");
        assert!(printed.contains("host"), "{printed}");
        assert!(!printed.contains("abc123"), "{printed}");
    }

    #[test]
    fn a_minted_token_is_43_url_safe_chars_and_differs_every_time() {
        let a = Secret::mint();
        let b = Secret::mint();
        assert_eq!(a.len(), 43);
        assert!(a
            .expose()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn expose_is_the_value() {
        assert_eq!(Secret::new("t".to_string()).expose(), "t");
    }
}
