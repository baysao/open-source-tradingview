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

pub mod bars_cache;
pub mod catalog;
pub mod clock;
pub mod config;
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
use config::{Config, ConfigError};
use feeds::{FeedHub, Limits};
use logbus::{Line, LogBus};
use sources::{Endpoints, Sources};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Everything a route needs. Every field is a cheap clone. 01-02 adds `clock` and `feeds`, 01-03
/// adds `logs`, Phase 03 adds `db`.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub catalog: Catalog,
    pub bars: BarsCache,
    pub clock: Clock,
    pub feeds: FeedHub,
    pub logs: LogBus,
    pub orderflow: orderflow::OrderFlow,
    limiter: net::RateLimiter,
    sources: Arc<Sources>,
}

impl AppState {
    /// The real upstream endpoints. No network I/O happens here.
    pub fn build(cfg: Config) -> Result<AppState, ConfigError> {
        let dir = cfg.log_dir.clone();
        Ok(AppState::assemble(cfg, Endpoints::default(), Some(&dir)))
    }

    /// Same as [`build`](Self::build) with the providers pointed at `endpoints` (tests use fakes).
    /// The log stays in memory here (no `hub.log`), so tests never write files.
    pub fn build_with(cfg: Config, endpoints: Endpoints) -> AppState {
        AppState::assemble(cfg, endpoints, None)
    }

    fn assemble(cfg: Config, endpoints: Endpoints, log_dir: Option<&std::path::Path>) -> AppState {
        let sources = Arc::new(Sources::new(endpoints));
        let catalog = Catalog::new(cfg.http_port);
        let bars = BarsCache::new(sources.clone());
        let clock = Clock::default();
        let limits = Limits {
            max_upstreams: cfg.feed_max_symbols,
            exempt_loopback: cfg.auth == config::AuthMode::Off,
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
