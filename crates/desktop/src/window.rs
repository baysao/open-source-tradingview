//! The window: the hosted hub, and no way out of it.
//!
//! Built in Rust rather than declared in `tauri.conf.json` because both navigation hooks only
//! exist on the builder (`on_navigation` at `tauri-2.12.7/src/webview/webview_window.rs:266`,
//! `on_new_window` at `:315`). `on_navigation` alone does not cover `window.open` and
//! `target=_blank`, and the hub frames engine pages, so both are wired.

use std::sync::atomic::Ordering;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config;
use crate::deeplink;
use crate::nav;
use crate::state::DesktopState;

/// The window label. The capability file targets exactly this label.
pub const LABEL: &str = "main";

pub fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let hub_url = format!("{}{}", config::PUBLIC_ORIGIN, config::HUB_PATH);
    tracing::info!(target: "desktop", "opening {hub_url}");

    let hook_app = app.clone();
    let win = WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::External(hub_url.parse().expect("the hub URL is a valid URL")),
    )
    .title("TradeX")
    .inner_size(1280.0, 820.0)
    .min_inner_size(720.0, 480.0)
    // The page surface is one function; see src/init.js.
    .initialization_script(include_str!("init.js"))
    .on_navigation(|url| {
        if nav::is_allowed(url) {
            true
        } else {
            // Hand it to the system browser, from Rust: the opener plugin's JS commands are not in
            // the capability, so the page cannot reach them. Returning false keeps the webview put.
            tracing::debug!(target: "desktop", "refusing in-window navigation to {url}");
            let _ = tauri_plugin_opener::open_url(url.as_str(), None::<&str>);
            false
        }
    })
    .on_new_window(|url, _features| {
        // A second window would be a webview with the same capability and no guard of its own.
        tracing::debug!(target: "desktop", "refusing a new window for {url}");
        let _ = tauri_plugin_opener::open_url(url.as_str(), None::<&str>);
        tauri::webview::NewWindowResponse::Deny
    })
    // Page readiness drives the deep-link queue. `on_page_load` exists on the builder only, which is
    // why the hook lives here rather than in `lib.rs` beside the other wiring.
    .on_page_load(move |w, payload| match payload.event() {
        tauri::webview::PageLoadEvent::Started => {
            // A document is being replaced; delivering into it would land in a page that is about
            // to be torn down.
            hook_app
                .state::<DesktopState>()
                .page_ready
                .store(false, Ordering::Release);
        }
        tauri::webview::PageLoadEvent::Finished => {
            let st = hook_app.state::<DesktopState>();
            deeplink::flush(&st, &deeplink::WindowEmitter(&w));
        }
    })
    .build()?;

    // Closing the window hides it; the tray's Quit is the way out (mockup: "Show and Quit").
    let handle = win.clone();
    win.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = handle.hide();
        }
    });

    Ok(win)
}
