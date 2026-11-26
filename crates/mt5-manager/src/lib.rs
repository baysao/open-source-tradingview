//! The MT5 bridge manager: the operator console for the host that runs the bridge and its tunnel.
//!
//! Two halves, split so the domain is testable without a GUI:
//!
//! - [`commands`] — the domain. Every system call goes through [`runner::CommandRunner`] and every
//!   HTTP call through [`http::Http`], so tests inject recorded outputs and no socket or live
//!   process is touched under `cargo test` (L-016's rule, D-036's test architecture).
//! - `run` (behind the non-default `gui` feature) — the Tauri wiring: five commands, in-memory
//!   secrets, one local window.
//!
//! What the webview may do is `capabilities/manager.json` and nothing else (AC-2).

pub mod commands;
pub mod http;
pub mod runner;
pub mod secret;
pub mod view;

#[cfg(feature = "gui")]
mod app;

#[cfg(feature = "gui")]
pub use app::run;
