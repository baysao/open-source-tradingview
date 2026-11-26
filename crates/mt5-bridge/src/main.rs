//! The bridge binary: env contract, the three MCP sessions, the offset task, and the axum server.
//!
//! Startup mirrors `bridge/mt5_bridge.py`: `bridge.env` beside the exe (never overriding real
//! environment variables), the same defaults, the same refusal when `BRIDGE_TOKEN` is missing, and
//! the same startup line. The offset file (`.bridge_offset`) is re-read on start and rewritten on
//! every accepted measurement.

mod http;
mod live;
mod mcp;
mod term;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;

use crate::live::{LivePolicy, PollerSet};
use crate::mcp::Mcp;
use crate::term::{offset_measure, Offset, Terminals};

/// The env contract, with the oracle's defaults (`bridge/mt5_bridge.py` lines 33-37).
#[derive(Clone, Debug)]
pub struct Config {
    pub bind: IpAddr,
    pub port: u16,
    pub token: String,
    pub mcp_url: String,
    pub mcp_token: String,
    pub state_dir: PathBuf,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

impl Config {
    /// Read the environment. `Err` is the exact line the oracle prints before it exits 1.
    pub fn from_env() -> Result<Self, String> {
        let token = var("BRIDGE_TOKEN")
            .map(|t| t.trim().to_string())
            .ok_or_else(|| {
                "BRIDGE_TOKEN is required (a random secret; the hub stores it encrypted)."
                    .to_string()
            })?;
        let bind = var("BRIDGE_BIND")
            .map(|b| {
                b.parse::<IpAddr>()
                    .map_err(|e| format!("BRIDGE_BIND is not an address: {e}"))
            })
            .transpose()?
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let port = match var("BRIDGE_PORT") {
            Some(p) => p
                .parse::<u16>()
                .map_err(|e| format!("BRIDGE_PORT is not a port: {e}"))?,
            None => 8797,
        };
        let state_dir = match var("BRIDGE_STATE_DIR") {
            Some(d) => PathBuf::from(d),
            // The oracle's default is the script's directory; the binary's own directory is the
            // equivalent, and it is what `host-install.ps1` writes `bridge.env` beside.
            None => std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from(".")),
        };
        Ok(Self {
            bind,
            port,
            token,
            mcp_url: var("MT5_MCP_URL").unwrap_or_else(|| "http://127.0.0.1:22346/mcp".to_string()),
            // The oracle raises only when a call is made, not at startup; keep that behavior.
            mcp_token: var("MT5_MCP_TOKEN").unwrap_or_default(),
            state_dir,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }

    pub fn offset_file(&self) -> PathBuf {
        self.state_dir.join(".bridge_offset")
    }
}

/// The oracle's `log()`: `YYYY-MM-DD HH:MM:SS UTC` then the fields, one line.
fn log(args: std::fmt::Arguments<'_>) {
    let stamp = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    println!("{stamp} {args}");
}

/// The offset task: measure immediately, then every 300 s (15 s until the first success), writing
/// the file on each accepted measurement. Failures are logged and the old offset stands.
async fn offset_task(misc: Arc<Mcp>, state: watch::Sender<Offset>, file: PathBuf) {
    loop {
        match offset_measure(&*misc).await {
            Ok(Some(m)) => {
                let current = state.borrow().clone();
                if m.offset_s != current.offset_s || current.measured_at == 0 {
                    log(format_args!(
                        "server time offset {:+.1} h",
                        m.offset_s as f64 / 3600.0
                    ));
                }
                let next = Offset {
                    offset_s: m.offset_s,
                    offset_src: "terminal server time".to_string(),
                    measured_at: m.measured_at,
                };
                let _ = std::fs::write(&file, m.offset_s.to_string());
                let _ = state.send(next);
            }
            Ok(None) => {}
            Err(e) => log(format_args!("offset check failed: {e}")),
        }
        let measured = state.borrow().measured_at != 0;
        tokio::time::sleep(Duration::from_secs(if measured { 300 } else { 15 })).await;
    }
}

/// Load `.bridge_offset`. A missing or malformed file keeps the default offset (the oracle's
/// `except (OSError, ValueError): pass`).
fn load_offset(file: &std::path::Path) -> Offset {
    match std::fs::read_to_string(file) {
        Ok(text) => match text.trim().parse::<i64>() {
            Ok(offset_s) => Offset {
                offset_s,
                offset_src: "last measured".to_string(),
                measured_at: 0,
            },
            Err(_) => Offset::default(),
        },
        Err(_) => Offset::default(),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // `bridge.env` beside the exe; existing variables win (the oracle reads the real environment).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = dotenvy::from_path(dir.join("bridge.env"));
        }
    }

    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(message) => {
            log(format_args!("{message}"));
            return ExitCode::FAILURE;
        }
    };

    let offset = load_offset(&cfg.offset_file());
    let (offset_tx, offset_rx) = watch::channel(offset);

    let make = |url: &str, token: &str| -> Result<Arc<Mcp>, String> {
        Mcp::new(url, token)
            .map(Arc::new)
            .map_err(|e| e.to_string())
    };
    let live_mcp = match make(&cfg.mcp_url, &cfg.mcp_token) {
        Ok(m) => m,
        Err(e) => {
            log(format_args!("{e}"));
            return ExitCode::FAILURE;
        }
    };
    let hist_mcp = match make(&cfg.mcp_url, &cfg.mcp_token) {
        Ok(m) => m,
        Err(e) => {
            log(format_args!("{e}"));
            return ExitCode::FAILURE;
        }
    };
    let misc_mcp = match make(&cfg.mcp_url, &cfg.mcp_token) {
        Ok(m) => m,
        Err(e) => {
            log(format_args!("{e}"));
            return ExitCode::FAILURE;
        }
    };

    tokio::spawn(offset_task(misc_mcp.clone(), offset_tx, cfg.offset_file()));

    // The two history-consuming sessions are distinct MCP sessions (the oracle's LIVE/HIST split).
    let pollers = Arc::new(PollerSet::new(
        live_mcp.clone(),
        offset_rx.clone(),
        LivePolicy::default(),
    ));
    let app = http::App {
        terminals: Terminals::new(live_mcp.clone(), hist_mcp.clone(), misc_mcp.clone()),
        pollers,
        token: cfg.token.clone(),
        offset: offset_rx,
    };

    let listener = match tokio::net::TcpListener::bind(cfg.addr()).await {
        Ok(l) => l,
        Err(e) => {
            log(format_args!("cannot listen on {}: {e}", cfg.addr()));
            return ExitCode::FAILURE;
        }
    };
    log(format_args!(
        "native MT5 bridge on {}:{}",
        cfg.bind, cfg.port
    ));
    match http::serve(listener, app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log(format_args!("server stopped: {e}"));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env tests run one at a time; the process environment is global state, and Rust runs
    /// tests in parallel threads by default.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap();
        let saved: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var(k).ok()))
            .collect();
        for (k, v) in vars {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        f();
        for (k, v) in saved {
            match v {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
    }

    #[test]
    fn a_missing_token_prints_the_oracles_line() {
        with_env(
            &[
                ("BRIDGE_TOKEN", None),
                ("BRIDGE_BIND", None),
                ("BRIDGE_PORT", None),
                ("MT5_MCP_URL", None),
                ("BRIDGE_STATE_DIR", None),
            ],
            || {
                let err = Config::from_env().expect_err("no token");
                assert_eq!(
                    err,
                    "BRIDGE_TOKEN is required (a random secret; the hub stores it encrypted)."
                );
            },
        );
    }

    #[test]
    fn the_defaults_match_the_python_oracle() {
        with_env(
            &[
                ("BRIDGE_TOKEN", Some("sekret")),
                ("BRIDGE_BIND", None),
                ("BRIDGE_PORT", None),
                ("MT5_MCP_URL", None),
                ("MT5_MCP_TOKEN", None),
                ("BRIDGE_STATE_DIR", None),
            ],
            || {
                let cfg = Config::from_env().expect("a config");
                assert_eq!(cfg.bind.to_string(), "127.0.0.1");
                assert_eq!(cfg.port, 8797);
                assert_eq!(cfg.mcp_url, "http://127.0.0.1:22346/mcp");
                assert_eq!(cfg.mcp_token, "");
                assert_eq!(cfg.token, "sekret");
                assert!(cfg.state_dir.is_absolute() || cfg.state_dir.as_os_str() == ".");
            },
        );
    }

    #[test]
    fn the_env_overrides_are_read() {
        with_env(
            &[
                ("BRIDGE_TOKEN", Some("  spaced  ")),
                ("BRIDGE_BIND", Some("0.0.0.0")),
                ("BRIDGE_PORT", Some("9123")),
                ("MT5_MCP_URL", Some("http://127.0.0.1:1/mcp")),
                ("BRIDGE_STATE_DIR", Some("C:/tmp/state")),
            ],
            || {
                let cfg = Config::from_env().expect("a config");
                assert_eq!(cfg.token, "spaced", "the oracle strips the token");
                assert_eq!(cfg.bind.to_string(), "0.0.0.0");
                assert_eq!(cfg.port, 9123);
                assert_eq!(cfg.mcp_url, "http://127.0.0.1:1/mcp");
                assert_eq!(cfg.state_dir, PathBuf::from("C:/tmp/state"));
                assert!(cfg.offset_file().ends_with(".bridge_offset"));
            },
        );
    }

    #[test]
    fn the_offset_file_round_trips_or_falls_back() {
        let dir = std::env::temp_dir().join(format!("mt5b-offset-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".bridge_offset");

        assert_eq!(load_offset(&file), Offset::default(), "missing file");

        std::fs::write(&file, "10800").unwrap();
        let loaded = load_offset(&file);
        assert_eq!(loaded.offset_s, 10800);
        assert_eq!(loaded.offset_src, "last measured");
        assert_eq!(loaded.measured_at, 0);

        std::fs::write(&file, "not a number").unwrap();
        assert_eq!(load_offset(&file), Offset::default(), "malformed file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
