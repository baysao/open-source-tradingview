//! `hub-server`: the Rust replacement for `python hub/server.py`. One port (default 8770) serves
//! the hub pages, `/api/*` and the live feed at `/ws`, bound to 127.0.0.1 (local mode, D-001).
//!
//! Start-up order: `.env` files (never overriding real environment variables), config, state,
//! logging (stdout plus the in-process log ring behind `/api/logs`), background tasks, listen.

use hub_core::config::Config;
use hub_core::logbus::RingLayer;
use hub_core::AppState;
use std::net::SocketAddr;
use std::process::ExitCode;
use tracing_subscriber::filter::{filter_fn, LevelFilter};
use tracing_subscriber::prelude::*;

/// Stdout shows what Python printed: `server` and `feed` lines at info and above, everything else
/// at warn and above, message only (no timestamp or level, like Python's `print`).
fn stdout_filter(meta: &tracing::Metadata<'_>) -> bool {
    match meta.target() {
        "server" | "feed" => *meta.level() <= tracing::Level::INFO,
        _ => *meta.level() <= tracing::Level::WARN,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Repo `.env` first, then `hub/.env`, as Python reads them; existing variables win.
    for path in [".env", "hub/.env"] {
        let _ = dotenvy::from_path(path);
    }
    let cfg = match Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("hub-server: {e}");
            return ExitCode::FAILURE;
        }
    };
    let addr = cfg.bind_addr();
    let port = cfg.http_port;
    let state = match AppState::start(cfg).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("hub-server: {e}");
            return ExitCode::FAILURE;
        }
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .without_time()
                .with_level(false)
                .with_target(false)
                .with_filter(filter_fn(stdout_filter)),
        )
        .with(RingLayer::new(state.logs.clone()).with_filter(LevelFilter::DEBUG))
        .init();

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(target: "server", "cannot listen on {addr}: {e} (is another hub already running?)");
            return ExitCode::FAILURE;
        }
    };
    let _background = state.spawn_background();
    tracing::info!(
        target: "server",
        "TradeX: open http://localhost:{port}/hub.html  (data on ws://localhost:{port}/ws)"
    );

    let app = hub_core::app(state).into_make_service_with_connect_info::<SocketAddr>();
    let served = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    match served {
        Ok(()) => {
            tracing::info!(target: "server", "Chart server stopped");
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(target: "server", "server error: {e}");
            ExitCode::FAILURE
        }
    }
}
