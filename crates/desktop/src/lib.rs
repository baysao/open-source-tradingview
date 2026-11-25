//! The TradeX desktop shell.
//!
//! One window on the hosted hub, one command the page may call, one deep-link hand-off, and a tray.
//! Everything the page can do is in `capabilities/hub.json`; everything the window may load is in
//! `src/nav.rs`.

pub mod config;
pub mod deeplink;
pub mod nav;
pub mod signin;
pub mod state;
pub mod tray;
pub mod window;

use tauri::Manager;

use tauri_plugin_deep_link::DeepLinkExt;
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "desktop=info".into()),
        )
        .init();

    tauri::Builder::default()
        // Order matters: single-instance first, so a second launch forwards its argv to this
        // process instead of starting another one. A deep link registered after this one reaches
        // `on_open_url`; registered before, it is dropped with no error.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(win) = app.get_webview_window(window::LABEL) {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
            // The plugin's own parser: it validates the scheme against `tauri.conf.json` and
            // updates `get_current()` before emitting, so one code path serves both the second
            // launch and the cold start.
            app.deep_link().handle_cli_arguments(argv.iter());
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![signin::start_sign_in])
        .setup(|app| {
            // Managed here, not passed to the window builder: a `State<'_, T>` parameter resolves
            // only against the app's managed store, so a state that is constructed but never
            // managed makes every call to the command fail with "state not managed for field `st`".
            // Found by the live invoke check in 06-01, not by the unit tests.
            app.manage(state::DesktopState::new());
            window::build(app.handle())?;
            tray::build(app.handle())?;

            // A dev run has no installer to write the OS registration, so register explicitly.
            // The bundled installer does it itself; registering twice is harmless.
            match app.deep_link().is_registered(config::DEEP_LINK_SCHEME) {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(e) = app.deep_link().register(config::DEEP_LINK_SCHEME) {
                        tracing::warn!(target: "desktop", "could not register the protocol: {e}");
                    } else {
                        tracing::info!(target: "desktop", "registered {}://", config::DEEP_LINK_SCHEME);
                    }
                }
                Err(e) => tracing::debug!(target: "desktop", "registration state unknown: {e}"),
            }

            // Cold start: the URL arrived as argv, so `get_current()` is the only way to see it.
            // Warm deliveries arrive on the event below.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for u in urls {
                    deliver_url(app.handle(), u.as_str());
                }
            }

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for u in event.urls() {
                    deliver_url(&handle, u.as_str());
                }
            });

            // Page readiness is wired on the window builder (`on_page_load` is builder-only), so
            // `window::build` owns the queue drain and nothing further is needed here.

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the TradeX desktop shell");
}

/// Parses one deep-link URL and delivers it, or logs the refusal and touches nothing.
fn deliver_url(app: &tauri::AppHandle, url: &str) {
    let Some(code) = deeplink::parse_code(url) else {
        // AC-5: one line, no event, no panic. The URL itself is not logged — only its scheme.
        tracing::warn!(target: "desktop", "deep link refused (did not parse as {}://auth?code=)",
            config::DEEP_LINK_SCHEME);
        return;
    };
    let st = app.state::<state::DesktopState>();
    let Some(win) = app.get_webview_window(window::LABEL) else {
        tracing::warn!(target: "desktop", "deep link arrived with no window; dropped");
        return;
    };
    deeplink::deliver(&st, &deeplink::WindowEmitter(&win), &code);
}
