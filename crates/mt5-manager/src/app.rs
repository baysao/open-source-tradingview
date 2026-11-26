//! The Tauri wiring: five commands, one window, in-memory secrets.
//!
//! This module is the ONLY place that names `Real`/`RealFs`, so the domain stays fake-driven and a
//! reader can audit the entire privileged surface in one file.
//!
//! Rules this file keeps:
//! - every command takes enums or plain strings that the domain re-validates; nothing here builds a
//!   shell string, and no command accepts one
//! - errors become [`CommandError`] verbatim; no `unwrap` on the invoke path turns a refusal into a
//!   panic the UI cannot show
//! - secrets live in a `Mutex` in app state and never touch the disk or a log line

use std::sync::Mutex;

use tauri::Manager;

use crate::commands::install::{
    self, InstallOutcome, InstallPaths, Progress, ProgressSink, RealDelay,
};
use crate::commands::status::{self, Endpoints};
use crate::commands::{self, Failure, SvcAction, SvcName};
use crate::http;
use crate::runner::{Real, RealFs};
use crate::secret::Secret;
use crate::view::{CommandError, PairView, StatusView};

/// Every command is `async` and runs its blocking half on a worker thread: Tauri executes a sync
/// command on the MAIN thread, so a 30 s pair or a minutes-long install would freeze the window.
/// The runtime thread that awaits here never touches the blocking call itself.
async fn blocking<T, F>(f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Failure> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::new(format!("the worker died: {e}"), None))?
        .map_err(CommandError::from)
}

/// Everything the window may read and the Rust side must remember.
///
/// Nothing is persisted: a restart means pairing again. That is the deliberate cost of never
/// writing a revoke secret to disk (AC-3).
#[derive(Default)]
pub struct AppState {
    paired: Mutex<Option<PairedState>>,
    /// The bridge token used for /health. Separate from `paired` because install needs it before any
    /// pairing (it polls the bridge the installer just started).
    bridge_token: Mutex<Option<Secret>>,
    /// The public edge URL, once known.
    edge_url: Mutex<Option<String>>,
    /// The running install's progress snapshot. Written by the install thread, read by the panel's
    /// poller: the webview has no event bus (AC-2), so push is not available and pull is the design.
    progress: std::sync::Arc<Mutex<Progress>>,
}

/// The `ProgressSink` the install thread gets: writes into the shared snapshot.
struct StateProgress(std::sync::Arc<Mutex<Progress>>);

impl ProgressSink for StateProgress {
    fn report(&self, p: Progress) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = p;
        }
    }
}

struct PairedState {
    hub_url: String,
    id: String,
    revoke_code: Secret,
    label: String,
}

type CmdResult<T> = Result<T, CommandError>;

fn real_http() -> CmdResult<http::Real> {
    http::Real::new().map_err(|e| CommandError::new(e, Some("Restart the app.")))
}

#[tauri::command]
pub async fn pair(
    st: tauri::State<'_, AppState>,
    hub_url: String,
    label: String,
    bridge_url: String,
    token: String,
    code: String,
) -> CmdResult<PairView> {
    let http = real_http()?;
    let token = Secret::new(token);
    let hub = hub_url.clone();
    let send_token = token.clone();
    let out =
        blocking(move || commands::pair(&http, &hub, &label, &bridge_url, &send_token, &code))
            .await?;
    let origin = status::normalized_hub(&hub_url)?;
    {
        let mut paired = st.paired.lock().expect("state lock");
        *paired = Some(PairedState {
            hub_url: origin,
            id: out.bridge.id.clone(),
            revoke_code: out.bridge.revoke_code.clone(),
            label: out.bridge.label.clone(),
        });
    }
    *st.bridge_token.lock().expect("state lock") = Some(token);
    Ok(PairView {
        id: out.bridge.id,
        label: out.bridge.label,
        url: out.bridge.url,
        symbol_count: out.bridge.symbol_count,
        connected: out.bridge.connected,
        revoke_code: out.bridge.revoke_code.expose().to_string(),
    })
}

/// Checks the MCP address and the token separately, WITHOUT installing anything, so the panel can
/// show two independent indicators while the operator types. The address probe carries no credential;
/// the token probe carries it. One command, two verdicts — not two commands, because the ACL surface
/// stays one entry wide.
#[tauri::command]
pub fn check_mcp(mcp_url: Option<String>, mcp_token: Option<String>) -> CmdResult<McpView> {
    let http = real_http()?;
    let url = install::normalize_mcp_url(mcp_url.as_deref())?;
    let token = mcp_token
        .filter(|t| !t.trim().is_empty())
        .map(Secret::new)
        .unwrap_or_else(|| Secret::new(String::new()));

    // Address first: its verdict is independent of the token, and the token verdict is meaningless
    // when the endpoint never answered.
    let (url_ok, url_detail) = match install::probe_mcp(&http, &url) {
        Ok(detail) => (true, format!("Port and path are reachable ({detail}).")),
        Err(why) => (false, why),
    };

    let (token_ok, token_detail) = if !url_ok {
        // Do not send the token to something that is not the terminal.
        (None, "Skipped: the address is not answering.".to_string())
    } else {
        match install::probe_token(&http, &url, &token) {
            Ok(status) => (
                Some(true),
                format!("The terminal accepted the token (answered {status})."),
            ),
            Err(install::TokenProbe::NoToken) => (
                None,
                "No token to test yet — paste it from MT5: Tools > Options > MCP.".to_string(),
            ),
            Err(install::TokenProbe::Refused(why)) => (Some(false), why),
            Err(install::TokenProbe::Unreachable(why)) => {
                // The address probe just passed, so this is a contradiction worth naming honestly.
                (Some(false), why)
            }
        }
    };

    Ok(McpView {
        url_ok,
        url_detail,
        token_ok,
        token_detail,
    })
}

/// The two verdicts the panel paints as its address and token dots. `token_ok` is `None` while no
/// token has been typed: neutral, not a failure.
#[derive(serde::Serialize)]
pub struct McpView {
    pub url_ok: bool,
    pub url_detail: String,
    pub token_ok: Option<bool>,
    pub token_detail: String,
}

/// The running install's progress. Pull-based on purpose: `core:event:*` is forbidden by AC-2, so the
/// panel polls this instead of the backend pushing.
#[tauri::command]
pub fn install_progress(st: tauri::State<'_, AppState>) -> Progress {
    st.progress.lock().map(|p| p.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn install(
    st: tauri::State<'_, AppState>,
    deploy_root: Option<String>,
    mcp_token: Option<String>,
    mcp_url: Option<String>,
) -> CmdResult<InstallOutcome> {
    let http = real_http()?;
    let mut paths = InstallPaths::default();
    if let Some(root) = deploy_root.filter(|r| !r.trim().is_empty()) {
        paths.deploy_root = root.trim().to_string();
    }
    let mcp = mcp_token.filter(|t| !t.trim().is_empty()).map(Secret::new);
    // Reset the snapshot so a second run cannot show the previous run's last step.
    if let Ok(mut slot) = st.progress.lock() {
        *slot = Progress::at(0, "starting", "preparing");
    }
    let sink = StateProgress(st.progress.clone());
    let out = install::install_stack_reporting(
        &Real,
        &RealFs,
        &http,
        &RealDelay,
        &paths,
        mcp.as_ref(),
        mcp_url.as_deref(),
        &sink,
    )
    .inspect_err(|e| {
        // A failed install must not leave the bar mid-flight forever.
        sink.report(Progress::finished(1).with_label(&format!("Stopped: {}", e.cause())));
    })?;
    // The poll used whatever token bridge.env holds; remember that one for /health.
    if out.health.is_some() {
        if let Ok(env) = std::fs::read_to_string(paths.env_file()) {
            if let Some((_, tok)) = install::parse_env(&env)
                .into_iter()
                .find(|(k, _)| k == "BRIDGE_TOKEN")
            {
                *st.bridge_token.lock().expect("state lock") = Some(Secret::new(tok));
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn control(svc: String, action: String) -> CmdResult<String> {
    let svc = SvcName::parse(&svc)?;
    let action = SvcAction::parse(&action)?;
    // The needle identifies the process for the kill fallback (L-034's command-line match).
    let needle = svc.image();
    blocking(move || commands::control(&Real, svc, action, needle).map(|o| o.message(svc))).await
}

#[tauri::command]
pub async fn status(
    st: tauri::State<'_, AppState>,
    bridge_port: Option<u16>,
) -> CmdResult<StatusView> {
    let http = real_http()?;
    let token = st
        .bridge_token
        .lock()
        .expect("state lock")
        .clone()
        .unwrap_or_else(|| Secret::new(String::new()));
    let endpoints = Endpoints {
        bridge_port: bridge_port.unwrap_or(install::DEFAULT_PORT),
        edge_url: st.edge_url.lock().expect("state lock").clone(),
    };
    let report = blocking(move || Ok(status::status(&http, &Real, &endpoints, &token))).await?;
    Ok(StatusView::from_report(&report))
}

#[tauri::command]
pub async fn revoke(st: tauri::State<'_, AppState>, hub_url: Option<String>) -> CmdResult<String> {
    let held = st.paired.lock().expect("state lock").take();
    let Some(held) = held else {
        return Err(CommandError::new(
            "no bridge is paired in this session",
            Some("Unregister it from the hub's Settings, or pair first."),
        ));
    };
    let http = real_http()?;
    let hub = hub_url
        .filter(|h| !h.trim().is_empty())
        .unwrap_or(held.hub_url);
    let id = held.id;
    let code = held.revoke_code;
    blocking(move || commands::revoke(&http, &hub, &id, &code)).await?;
    Ok(format!("{} unregistered", held.label))
}

/// The app entry point: one window, loaded from `ui/`, holding the five commands above.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mt5_manager=info".into()),
        )
        .init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            pair,
            install,
            control,
            status,
            revoke,
            check_mcp,
            install_progress
        ])
        .setup(|app| {
            // Managed here (not passed to a window builder): a `State<'_, T>` resolves only against
            // the app's managed store, and an unmanaged state fails every call (crates/desktop's
            // lib.rs:44 records the same trap found live in 06-01).
            app.manage(AppState::default());

            // One local window, built in Rust: `frontendDist` is `ui/`, and a single window means
            // the capability's `windows: ["main"]` names everything there is.
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("BridgeManager")
            .inner_size(900.0, 760.0)
            .min_inner_size(560.0, 480.0)
            .visible(false)
            .build()?;
            if let Some(win) = app.get_webview_window("main") {
                // Shown after first paint: an empty white rectangle reads as a broken app.
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(120));
                    let _ = win.show();
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the BridgeManager");
}
