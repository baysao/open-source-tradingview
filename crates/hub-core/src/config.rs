//! Process configuration. `Config::from_env` is the only reader of environment variables.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::{env, fmt};

/// How users are authenticated. Phase 01 has exactly one mode: local, no accounts (D-001).
/// Phase 03 adds the Google variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    /// Local mode: no sign-in, no database, bound to 127.0.0.1 only.
    Off,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub http_port: u16,
    pub static_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Order-flow trade cache (D-008): daily files and complete REST hours. `HUB_CACHE_DIR`.
    pub cache_dir: PathBuf,
    pub public_origin: String,
    pub auth: AuthMode,
    /// Most distinct `(source, symbol)` upstreams open at once (D-002).
    pub feed_max_symbols: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Invalid {
        key: &'static str,
        value: String,
    },
    /// `AUTH_ENABLED=true` asks for a mode Phase 01 does not have.
    AuthNotAvailable,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Invalid { key, value } => write!(f, "invalid value for {key}: {value:?}"),
            ConfigError::AuthNotAvailable => f.write_str(
                "AUTH_ENABLED=true needs the sign-in phase; only local mode exists in this build",
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads `HUB_HTTP_PORT`, `HUB_STATIC_DIR`, `HUB_LOG_DIR`, `PUBLIC_ORIGIN`, `AUTH_ENABLED`,
    /// `FEED_MAX_SYMBOLS`. Defaults: 8770, `hub`, `hub/logs`, `http://127.0.0.1:<port>`, false, 400.
    pub fn from_env() -> Result<Config, ConfigError> {
        Config::from_lookup(|k| env::var(k).ok())
    }

    /// Same as `from_env`, with an injectable lookup (tests).
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let http_port = match get("HUB_HTTP_PORT") {
            Some(v) => v.trim().parse::<u16>().map_err(|_| ConfigError::Invalid {
                key: "HUB_HTTP_PORT",
                value: v,
            })?,
            None => 8770,
        };
        let auth = match get("AUTH_ENABLED").map(|v| v.trim().to_ascii_lowercase()) {
            None => AuthMode::Off,
            Some(v) if v.is_empty() || v == "0" || v == "false" || v == "no" => AuthMode::Off,
            Some(v) if v == "1" || v == "true" || v == "yes" => {
                return Err(ConfigError::AuthNotAvailable)
            }
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
            public_origin: get("PUBLIC_ORIGIN")
                .map(|v| v.trim_end_matches('/').to_string())
                .unwrap_or_else(|| format!("http://127.0.0.1:{http_port}")),
            auth,
            feed_max_symbols,
        })
    }

    /// The address to bind. Local mode binds loopback only, by construction (D-001).
    pub fn bind_addr(&self) -> SocketAddr {
        match self.auth {
            AuthMode::Off => SocketAddr::from((Ipv4Addr::LOCALHOST, self.http_port)),
        }
    }
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

    #[test]
    fn defaults() {
        let c = cfg(&[]).unwrap();
        assert_eq!(c.http_port, 8770);
        assert_eq!(c.public_origin, "http://127.0.0.1:8770");
        assert_eq!(c.feed_max_symbols, 400);
        assert_eq!(c.bind_addr().to_string(), "127.0.0.1:8770");
    }

    #[test]
    fn auth_enabled_is_refused_in_local_build() {
        assert_eq!(
            cfg(&[("AUTH_ENABLED", "true")]).unwrap_err(),
            ConfigError::AuthNotAvailable
        );
        assert!(cfg(&[("AUTH_ENABLED", "false")]).is_ok());
    }

    #[test]
    fn bad_port_is_an_error() {
        assert!(matches!(
            cfg(&[("HUB_HTTP_PORT", "abc")]),
            Err(ConfigError::Invalid { .. })
        ));
    }
}
