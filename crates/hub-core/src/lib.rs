#![allow(clippy::disallowed_methods)] // denied inside `orderflow` only (float order)

//! The TradeX hub server as a library. There is one entry point, [`app`]; every host (the
//! `hub-server` binary, the contract suite, later the hosted site) serves this router.
//!
//! ```text
//! let cfg   = Config::from_env()?;              // the only reader of env vars
//! let state = AppState::build(cfg)?;            // no network I/O; safe in tests
//! let _bg   = state.spawn_background();         // symbol refresh loops
//! hub_core::serve(listener, state).await?;      // app(state) with ConnectInfo
//! ```

pub mod auth;
pub mod bars_cache;
pub mod catalog;
pub mod clock;
pub mod config;
pub mod db;
pub mod feedlog;
pub mod feeds;
pub mod logbus;
pub mod market;
pub mod net;
pub mod orderflow;
pub mod routes;
pub mod sources;
pub mod static_files;

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use bars_cache::BarsCache;
use catalog::Catalog;
use clock::Clock;
use config::{AuthMode, Config, ConfigError};
use db::Db;
use feeds::{FeedHub, Limits};
use logbus::{Line, LogBus};
use sources::{Endpoints, Sources};
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Everything a route needs. Every field is a cheap clone. 01-02 adds `clock` and `feeds`, 01-03
/// adds `logs`, 03-02 adds `auth` (present only in hosted mode).
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub catalog: Catalog,
    pub bars: BarsCache,
    pub clock: Clock,
    pub feeds: FeedHub,
    pub logs: LogBus,
    pub orderflow: orderflow::OrderFlow,
    pub auth: Option<auth::Auth>,
    limiter: net::RateLimiter,
    sources: Arc<Sources>,
}

/// Why [`AppState::start`] could not bring the state up. `Display` names the missing environment
/// variable, the invalid one, or the failing migration; secret values never appear.
#[derive(Debug)]
pub enum StartError {
    /// The environment was already wrong (`Config::from_lookup` failed).
    Config(ConfigError),
    /// The database could not be opened or migrated.
    Db(db::DbError),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Config(e) => write!(f, "{e}"),
            StartError::Db(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StartError::Config(e) => Some(e),
            StartError::Db(e) => Some(e),
        }
    }
}

impl From<ConfigError> for StartError {
    fn from(e: ConfigError) -> StartError {
        StartError::Config(e)
    }
}

impl From<db::DbError> for StartError {
    fn from(e: db::DbError) -> StartError {
        StartError::Db(e)
    }
}

impl AppState {
    /// The real upstream endpoints. No network I/O happens here.
    ///
    /// Hosted mode is not served by this constructor: it needs the database, so it goes through
    /// [`AppState::start`] (which opens and migrates). `build` refuses it with the instruction.
    pub fn build(cfg: Config) -> Result<AppState, ConfigError> {
        if !cfg.auth.is_local() {
            return Err(ConfigError::Invalid {
                key: "AUTH_ENABLED",
                value: "use AppState::start".into(),
            });
        }
        let dir = cfg.log_dir.clone();
        Ok(AppState::assemble(
            cfg,
            Endpoints::default(),
            Some(&dir),
            None,
        ))
    }

    /// Local mode with the providers pointed at `endpoints` (tests use fakes).
    /// The log stays in memory here (no `hub.log`), so tests never write files.
    pub fn build_with(cfg: Config, endpoints: Endpoints) -> AppState {
        AppState::assemble(cfg, endpoints, None, None)
    }

    /// Local mode with a fully constructed [`Auth`] (test suites: the in-memory Idp and the manual
    /// clock). Panics on a local config: only tests that deliberately build hosted state call it.
    #[cfg(feature = "test-idp")]
    pub fn build_with_auth(cfg: Config, endpoints: Endpoints, auth: auth::Auth) -> AppState {
        assert!(
            !cfg.auth.is_local(),
            "build_with_auth is for hosted configs"
        );
        AppState::assemble(cfg, endpoints, None, Some(auth))
    }

    /// The one entry point that may serve hosted mode: local mode never touches the network,
    /// hosted mode opens and migrates the account database first, then builds the [`auth::Auth`]
    /// service with the production Idp and the wall clock.
    pub async fn start(cfg: Config) -> Result<AppState, StartError> {
        if cfg.auth.is_local() {
            let dir = cfg.log_dir.clone();
            return Ok(AppState::assemble(
                cfg,
                Endpoints::default(),
                Some(&dir),
                None,
            ));
        }
        let AuthMode::Google(settings) = &cfg.auth else {
            unreachable!("not local means hosted")
        };
        let origin = cfg.public_origin.clone();
        let redirect_uri = format!("{}/auth/google/callback", origin.trim_end_matches('/'));
        let db = Db::open(&settings.database_url).await?;
        let auth = auth::Auth::new(
            settings.clone(),
            &origin,
            db,
            auth::AuthDeps::production(settings, redirect_uri),
        );
        let dir = cfg.log_dir.clone();
        tracing::info!(
            target: "server",
            "auth: hosted mode on (Google sign-in, database connected)"
        );
        Ok(AppState::assemble(
            cfg,
            Endpoints::default(),
            Some(&dir),
            Some(auth),
        ))
    }

    fn assemble(
        cfg: Config,
        endpoints: Endpoints,
        log_dir: Option<&std::path::Path>,
        auth: Option<auth::Auth>,
    ) -> AppState {
        let sources = Arc::new(Sources::new(endpoints));
        let catalog = Catalog::new(cfg.http_port);
        let bars = BarsCache::new(sources.clone());
        let clock = Clock::default();
        let limits = Limits {
            max_upstreams: cfg.feed_max_symbols,
            exempt_loopback: cfg.auth.is_local(),
            ..Limits::default()
        };
        let feeds = FeedHub::new(
            limits,
            bars.clone(),
            clock.clone(),
            sources.endpoints().clone(),
        );
        let boot = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let logs = LogBus::new(log_dir, boot, clock.clone());
        // The trade cache only in the production build (`build`), so tests write no files (L-008).
        let orderflow = orderflow::OrderFlow::new(
            bars.clone(),
            sources.endpoints(),
            clock::WallClock::System,
            log_dir.map(|_| cfg.cache_dir.clone()),
        );
        logs.push(Line::new(
            "server",
            "info",
            format!("Chart server started (pid {})", std::process::id()),
        ));
        AppState {
            cfg: Arc::new(cfg),
            catalog,
            bars,
            clock,
            feeds,
            logs,
            orderflow,
            auth,
            limiter: net::RateLimiter::new(20.0, 40.0),
            sources,
        }
    }

    /// Starts the background tasks (symbol refresh loops). Dropping the handle stops them.
    pub fn spawn_background(&self) -> BackgroundTasks {
        let mut tasks = self.catalog.spawn_refresh(self.sources.clone());
        tasks.push(self.feeds.spawn_sweeper());
        tasks.push(feedlog::spawn(
            self.logs.clone(),
            self.feeds.clone(),
            self.catalog.clone(),
            self.bars.clone(),
        ));
        // Hosted mode only: expire dead sessions and oauth tickets once an hour (03-02).
        if let Some(auth) = self.auth.clone() {
            tasks.push(tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                tick.tick().await; // the first tick completes immediately
                loop {
                    tick.tick().await;
                    match auth.sweep_expired().await {
                        Ok(n) if n > 0 => tracing::info!(
                            target: "server",
                            "auth: expired {} session(s) and ticket(s)",
                            n
                        ),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(target: "server", "auth: sweep failed: {e:?}"),
                    }
                }
            }));
        }
        let clock = self.clock.clone();
        tasks.push(tokio::spawn(async move {
            match clock.measure("time.windows.com:123").await {
                Ok(ms) => tracing::info!(target: "server", "Clock offset vs time.windows.com: {ms:+} ms (corrected in lag numbers)"),
                Err(e) => tracing::warn!(target: "server", "clock offset failed: {e}"),
            }
        }));
        BackgroundTasks(tasks)
    }
}

/// Background task handles; dropping this aborts them.
pub struct BackgroundTasks(Vec<JoinHandle<()>>);

impl Drop for BackgroundTasks {
    fn drop(&mut self) {
        for h in &self.0 {
            h.abort();
        }
    }
}

/// The only router. Unknown `/api/*` paths and refused static paths answer 404
/// `{"error":"not found"}`.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/api/status", get(routes::market::status))
        .route("/api/symbols", get(routes::market::symbols))
        .route("/api/instruments", get(routes::market::instruments))
        .route("/api/bars", get(routes::market::bars))
        .route("/api/health", get(routes::health::health))
        .route("/ws", get(routes::ws::upgrade))
        .route("/api/logs", get(routes::logs::logs))
        .route(
            "/api/orderflow/delta",
            get(
                |State(s): State<AppState>, Query(q): Query<HashMap<String, String>>| {
                    routes::orderflow::serve(orderflow::Kind::Delta, s, q)
                },
            ),
        )
        .route(
            "/api/orderflow/profile",
            get(
                |State(s): State<AppState>, Query(q): Query<HashMap<String, String>>| {
                    routes::orderflow::serve(orderflow::Kind::Profile, s, q)
                },
            ),
        )
        .route(
            "/api/orderflow/footprint",
            get(
                |State(s): State<AppState>, Query(q): Query<HashMap<String, String>>| {
                    routes::orderflow::serve(orderflow::Kind::Footprint, s, q)
                },
            ),
        )
        .route(
            "/api/logs/journal",
            axum::routing::post(routes::logs::journal),
        )
        .fallback(fallback)
        .layer(axum::middleware::from_fn_with_state(
            state.limiter.clone(),
            net::rate_limit,
        ))
        .layer(axum::middleware::from_fn(net::security_headers))
        .with_state(state)
}

async fn fallback(State(st): State<AppState>, req: Request<Body>) -> Response {
    if req.uri().path().starts_with("/api/") {
        return routes::api_error(StatusCode::NOT_FOUND, "not found");
    }
    static_files::serve(&st.cfg.static_dir, req).await
}

/// Serves [`app`] on `listener` with peer addresses available to handlers.
pub async fn serve(listener: TcpListener, state: AppState) -> std::io::Result<()> {
    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

#[cfg(test)]
mod build_tests {
    use super::*;

    #[test]
    fn build_refuses_hosted_mode() {
        let vars = [
            ("AUTH_ENABLED", "true"),
            ("DATABASE_URL", "postgres://127.0.0.1:1/x"),
            ("GOOGLE_CLIENT_ID", "id"),
            ("GOOGLE_CLIENT_SECRET", "s"),
            (
                "VTV_SECRET_KEY",
                "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE",
            ),
            ("PUBLIC_ORIGIN", "https://vtradingview.com"),
        ];
        let cfg = Config::from_lookup(|k| {
            vars.iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        })
        .expect("hosted config parses");
        assert!(matches!(
            AppState::build(cfg),
            Err(ConfigError::Invalid {
                key: "AUTH_ENABLED",
                ..
            })
        ));
        // The local-mode half is not built here: `build` opens hub/logs/hub.log (L-008).
    }
}
