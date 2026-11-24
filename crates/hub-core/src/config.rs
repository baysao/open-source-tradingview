//! Process configuration. `Config::from_env` is the only reader of environment variables.

use base64::Engine as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::{env, fmt};

/// How users are authenticated (D-001, Phase 03).
#[derive(Clone, Debug)]
pub enum AuthMode {
    /// Local mode: no sign-in, no database, bound to 127.0.0.1 only.
    Off,
    /// Hosted mode: Google sign-in over the account database (`AUTH_ENABLED=true`).
    Google(Arc<AuthSettings>),
}

impl AuthMode {
    /// True in local mode (no accounts).
    pub fn is_local(&self) -> bool {
        matches!(self, AuthMode::Off)
    }
}

/// A secret value from the environment (a database URL with its password, a client secret).
/// `Debug` prints `<redacted>`; there is no `Display` and no `Serialize`, so it cannot reach a log
/// or a response by accident. Only this crate reads the value.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Secret {
        Secret(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// `VTV_SECRET_KEY`: at least 32 random bytes (base64url). Built only by the parser; `Debug` redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(Vec<u8>);

impl SecretKey {
    #[allow(dead_code)] // the HKDF keys are derived from it (03-02)
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where the identity provider lives. Google's constants in release builds; the `GOOGLE_*`
/// variables override them only with the `test-idp` feature (spec R21).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdpEndpoints {
    pub auth_url: String,
    pub token_url: String,
    pub jwks_url: String,
    pub issuers: Vec<String>,
}

impl IdpEndpoints {
    pub fn google() -> IdpEndpoints {
        IdpEndpoints {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            jwks_url: "https://www.googleapis.com/oauth2/v3/certs".into(),
            issuers: vec![
                "accounts.google.com".into(),
                "https://accounts.google.com".into(),
            ],
        }
    }
}

/// Everything hosted mode needs. `Debug` shows no secret.
#[derive(Debug)]
pub struct AuthSettings {
    /// `DATABASE_URL` (may hold a password).
    pub database_url: Secret,
    /// `GOOGLE_CLIENT_ID` (public: the page gets it from `/auth/me`, D-017).
    pub client_id: String,
    /// `GOOGLE_CLIENT_SECRET`.
    pub client_secret: Secret,
    /// `VTV_SECRET_KEY`.
    pub secret_key: SecretKey,
    /// `ADMIN_EMAILS`: trimmed, lowercased, empty entries dropped; empty by default.
    pub admin_emails: Vec<String>,
    pub idp: IdpEndpoints,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub http_port: u16,
    pub static_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Order-flow trade cache (D-008): daily files and complete REST hours. `HUB_CACHE_DIR`.
    pub cache_dir: PathBuf,
    /// The site's own origin. With auth on it is `https://...` (test-idp builds also accept
    /// `http://127.0.0.1:<port>`), and it is the only `Origin` a write may carry (D-010).
    pub public_origin: String,
    pub auth: AuthMode,
    /// Most distinct `(source, symbol)` upstreams open at once (D-002).
    pub feed_max_symbols: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// A value that does not parse. For a secret variable `value` is `<redacted>`.
    Invalid { key: &'static str, value: String },
    /// Required variables that are absent or empty, every one at once, in the spec's order.
    Missing { keys: Vec<&'static str> },
    /// A hosted config given to a build that cannot serve it yet (removed by 03-02).
    AuthNotAvailable,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Invalid { key, value } => write!(f, "invalid value for {key}: {value:?}"),
            ConfigError::Missing { keys } => write!(f, "missing {}", keys.join(", ")),
            ConfigError::AuthNotAvailable => f.write_str(
                "AUTH_ENABLED=true needs plan 03-02; this build has the database layer only",
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Variables hosted mode requires, in the order a `Missing` error names them.
const REQUIRED: [&str; 5] = [
    "DATABASE_URL",
    "GOOGLE_CLIENT_ID",
    "GOOGLE_CLIENT_SECRET",
    "VTV_SECRET_KEY",
    "PUBLIC_ORIGIN",
];

impl Config {
    /// Reads `HUB_HTTP_PORT`, `HUB_STATIC_DIR`, `HUB_LOG_DIR`, `HUB_CACHE_DIR`, `PUBLIC_ORIGIN`,
    /// `AUTH_ENABLED`, `FEED_MAX_SYMBOLS`, and with `AUTH_ENABLED=true` also `DATABASE_URL`,
    /// `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `VTV_SECRET_KEY`, `ADMIN_EMAILS`.
    /// Defaults: 8770, `hub`, `hub/logs`, `hub/.cache/orderflow`, `http://127.0.0.1:<port>`
    /// (local mode only), false, 400.
    pub fn from_env() -> Result<Config, ConfigError> {
        Config::from_lookup(|k| env::var(k).ok())
    }

    /// Same as `from_env`, with an injectable lookup (tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        // An empty value counts as absent.
        let get = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        let http_port = match get("HUB_HTTP_PORT") {
            Some(v) => v.trim().parse::<u16>().map_err(|_| ConfigError::Invalid {
                key: "HUB_HTTP_PORT",
                value: v,
            })?,
            None => 8770,
        };
        let hosted = match get("AUTH_ENABLED").map(|v| v.trim().to_ascii_lowercase()) {
            None => false,
            Some(v) if v == "0" || v == "false" || v == "no" => false,
            Some(v) if v == "1" || v == "true" || v == "yes" => true,
            Some(v) => {
                return Err(ConfigError::Invalid {
                    key: "AUTH_ENABLED",
                    value: v,
                })
            }
        };
        let feed_max_symbols = match get("FEED_MAX_SYMBOLS") {
            Some(v) => match v.trim().parse::<usize>() {
                Ok(n) if n > 0 => n,
                _ => {
                    return Err(ConfigError::Invalid {
                        key: "FEED_MAX_SYMBOLS",
                        value: v,
                    })
                }
            },
            None => 400,
        };
        let origin = get("PUBLIC_ORIGIN").map(|v| v.trim().trim_end_matches('/').to_string());
        let (auth, public_origin) = if hosted {
            let missing: Vec<&'static str> = REQUIRED
                .iter()
                .copied()
                .filter(|k| get(k).is_none())
                .collect();
            if !missing.is_empty() {
                return Err(ConfigError::Missing { keys: missing });
            }
            let origin = origin.unwrap_or_default();
            if !hosted_origin_ok(&origin) {
                return Err(ConfigError::Invalid {
                    key: "PUBLIC_ORIGIN",
                    value: origin,
                });
            }
            let settings = AuthSettings {
                database_url: Secret::new(get("DATABASE_URL").unwrap_or_default().trim()),
                client_id: get("GOOGLE_CLIENT_ID")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                client_secret: Secret::new(get("GOOGLE_CLIENT_SECRET").unwrap_or_default().trim()),
                secret_key: parse_secret_key(&get("VTV_SECRET_KEY").unwrap_or_default())?,
                admin_emails: get("ADMIN_EMAILS")
                    .map(|v| {
                        v.split(',')
                            .map(|e| e.trim().to_ascii_lowercase())
                            .filter(|e| !e.is_empty())
                            .collect()
                    })
                    .unwrap_or_default(),
                idp: idp_endpoints(&get),
            };
            (AuthMode::Google(Arc::new(settings)), origin)
        } else {
            (
                AuthMode::Off,
                origin.unwrap_or_else(|| format!("http://127.0.0.1:{http_port}")),
            )
        };
        Ok(Config {
            http_port,
            static_dir: get("HUB_STATIC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("hub")),
            log_dir: get("HUB_LOG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("hub/logs")),
            cache_dir: get("HUB_CACHE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("hub/.cache/orderflow")),
            public_origin,
            auth,
            feed_max_symbols,
        })
    }

    /// The address to bind: loopback in both modes. Local mode is loopback by construction
    /// (D-001); the hosted site is reached only through the tunnel, which connects from 127.0.0.1.
    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, self.http_port))
    }
}

/// `https://host[:port]` with no path; test-idp builds also accept `http://127.0.0.1:<port>`.
fn hosted_origin_ok(origin: &str) -> bool {
    let host_only = |rest: &str| !rest.is_empty() && !rest.contains('/') && !rest.contains(' ');
    if let Some(rest) = origin.strip_prefix("https://") {
        return host_only(rest);
    }
    #[cfg(feature = "test-idp")]
    if let Some(port) = origin.strip_prefix("http://127.0.0.1:") {
        return port.parse::<u16>().is_ok();
    }
    false
}

/// base64url (padding optional) decoding to at least 32 bytes. The value never appears in an error.
fn parse_secret_key(v: &str) -> Result<SecretKey, ConfigError> {
    let invalid = || ConfigError::Invalid {
        key: "VTV_SECRET_KEY",
        value: "<redacted>".into(),
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(v.trim().trim_end_matches('='))
        .map_err(|_| invalid())?;
    if bytes.len() < 32 {
        return Err(invalid());
    }
    Ok(SecretKey(bytes))
}

#[cfg(feature = "test-idp")]
fn idp_endpoints(get: &impl Fn(&str) -> Option<String>) -> IdpEndpoints {
    let g = IdpEndpoints::google();
    IdpEndpoints {
        auth_url: get("GOOGLE_AUTH_URL").unwrap_or(g.auth_url),
        token_url: get("GOOGLE_TOKEN_URL").unwrap_or(g.token_url),
        jwks_url: get("GOOGLE_JWKS_URL").unwrap_or(g.jwks_url),
        issuers: get("GOOGLE_ISSUER").map(|i| vec![i]).unwrap_or(g.issuers),
    }
}

/// Release builds use Google's endpoints whatever the environment says (R21).
#[cfg(not(feature = "test-idp"))]
fn idp_endpoints(_get: &impl Fn(&str) -> Option<String>) -> IdpEndpoints {
    IdpEndpoints::google()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(pairs: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|k| m.get(k).cloned())
    }

    /// 32 bytes of 0x01, base64url without padding.
    const KEY32: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";
    /// 31 bytes.
    const KEY31: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQ";

    fn hosted(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        let mut v = vec![
            ("AUTH_ENABLED", "true"),
            (
                "DATABASE_URL",
                "postgres://vtv:hunter2pass@127.0.0.1:55432/main",
            ),
            ("GOOGLE_CLIENT_ID", "client-123.apps.googleusercontent.com"),
            ("GOOGLE_CLIENT_SECRET", "gsecret-xyz"),
            ("VTV_SECRET_KEY", KEY32),
            ("PUBLIC_ORIGIN", "https://vtradingview.com"),
        ];
        for (k, val) in extra {
            v.retain(|(kk, _)| kk != k);
            v.push((k, val));
        }
        v
    }

    fn settings(c: &Config) -> &AuthSettings {
        match &c.auth {
            AuthMode::Google(s) => s,
            AuthMode::Off => panic!("expected hosted mode"),
        }
    }

    #[test]
    fn defaults() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.http_port, 8770);
        assert_eq!(c.public_origin, "http://127.0.0.1:8770");
        assert_eq!(c.feed_max_symbols, 400);
        assert!(c.auth.is_local());
        assert_eq!(c.bind_addr().to_string(), "127.0.0.1:8770");
    }

    #[test]
    fn local_mode_ignores_hosted_variables() {
        assert!(cfg(&[("AUTH_ENABLED", "false")]).unwrap().auth.is_local());
        assert!(cfg(&[("AUTH_ENABLED", "")]).unwrap().auth.is_local());
        assert!(cfg(&[("VTV_SECRET_KEY", "x")]).unwrap().auth.is_local());
        assert!(matches!(
            cfg(&[("AUTH_ENABLED", "maybe")]),
            Err(ConfigError::Invalid {
                key: "AUTH_ENABLED",
                ..
            })
        ));
    }

    #[test]
    fn hosted_names_every_missing_variable_at_once() {
        let e = cfg(&[
            ("AUTH_ENABLED", "true"),
            ("PUBLIC_ORIGIN", "https://vtradingview.com"),
        ])
        .unwrap_err();
        assert_eq!(
            e,
            ConfigError::Missing {
                keys: vec![
                    "DATABASE_URL",
                    "GOOGLE_CLIENT_ID",
                    "GOOGLE_CLIENT_SECRET",
                    "VTV_SECRET_KEY"
                ]
            }
        );
        assert_eq!(
            e.to_string(),
            "missing DATABASE_URL, GOOGLE_CLIENT_ID, GOOGLE_CLIENT_SECRET, VTV_SECRET_KEY"
        );
        // An empty value is missing too; PUBLIC_ORIGIN is required when hosted.
        let e = cfg(&hosted(&[
            ("GOOGLE_CLIENT_ID", "  "),
            ("PUBLIC_ORIGIN", ""),
        ]))
        .unwrap_err();
        assert_eq!(
            e,
            ConfigError::Missing {
                keys: vec!["GOOGLE_CLIENT_ID", "PUBLIC_ORIGIN"]
            }
        );
    }

    #[test]
    fn hosted_parses_and_binds_loopback() {
        let c = cfg(&hosted(&[("ADMIN_EMAILS", " A@x.io, b@y.io ,,")])).unwrap();
        let s = settings(&c);
        assert_eq!(c.public_origin, "https://vtradingview.com");
        assert_eq!(s.admin_emails, vec!["a@x.io", "b@y.io"]);
        assert_eq!(s.client_id, "client-123.apps.googleusercontent.com");
        assert_eq!(s.secret_key.bytes(), &[1u8; 32][..]);
        assert_eq!(c.bind_addr().to_string(), "127.0.0.1:8770");
        assert!(!c.auth.is_local());
        // Trailing slash dropped; padding accepted.
        let c = cfg(&hosted(&[
            ("PUBLIC_ORIGIN", "https://vtradingview.com/"),
            (
                "VTV_SECRET_KEY",
                "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
            ),
        ]))
        .unwrap();
        assert_eq!(c.public_origin, "https://vtradingview.com");
        assert!(settings(&cfg(&hosted(&[])).unwrap())
            .admin_emails
            .is_empty());
    }

    #[test]
    fn hosted_refuses_http_origin_and_short_key() {
        assert_eq!(
            cfg(&hosted(&[("PUBLIC_ORIGIN", "http://example.com")])).unwrap_err(),
            ConfigError::Invalid {
                key: "PUBLIC_ORIGIN",
                value: "http://example.com".into()
            }
        );
        assert!(cfg(&hosted(&[("PUBLIC_ORIGIN", "https://a.com/path")])).is_err());
        for bad in [KEY31, "not base64 !!", "AQ"] {
            let e = cfg(&hosted(&[("VTV_SECRET_KEY", bad)])).unwrap_err();
            assert_eq!(
                e,
                ConfigError::Invalid {
                    key: "VTV_SECRET_KEY",
                    value: "<redacted>".into()
                },
                "{bad}"
            );
            assert!(!e.to_string().contains(bad));
        }
    }

    #[test]
    fn debug_shows_no_secret() {
        let c = cfg(&hosted(&[])).unwrap();
        for text in [format!("{c:?}"), format!("{:?}", settings(&c))] {
            for secret in ["hunter2pass", "gsecret-xyz", KEY32, "AQEB", "[1, 1, 1"] {
                assert!(!text.contains(secret), "Debug leaks {secret}: {text}");
            }
            assert!(text.contains("<redacted>"));
        }
    }

    #[test]
    fn idp_urls_follow_the_feature() {
        let c = cfg(&hosted(&[
            ("GOOGLE_JWKS_URL", "http://127.0.0.1:9/jwks"),
            ("GOOGLE_ISSUER", "http://127.0.0.1:9"),
        ]))
        .unwrap();
        let idp = &settings(&c).idp;
        if cfg!(feature = "test-idp") {
            assert_eq!(idp.jwks_url, "http://127.0.0.1:9/jwks");
            assert_eq!(idp.issuers, vec!["http://127.0.0.1:9".to_string()]);
            assert_eq!(idp.token_url, IdpEndpoints::google().token_url);
        } else {
            assert_eq!(idp, &IdpEndpoints::google());
        }
    }

    #[test]
    fn test_idp_origin_only_with_the_feature() {
        let r = cfg(&hosted(&[("PUBLIC_ORIGIN", "http://127.0.0.1:8770")]));
        assert_eq!(r.is_ok(), cfg!(feature = "test-idp"));
        assert!(cfg(&hosted(&[("PUBLIC_ORIGIN", "http://localhost:8770")])).is_err());
    }

    #[test]
    fn bad_port_is_an_error() {
        assert!(matches!(
            cfg(&[("HUB_HTTP_PORT", "abc")]),
            Err(ConfigError::Invalid { .. })
        ));
    }
}
