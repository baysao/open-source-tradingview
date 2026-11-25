//! `start_sign_in`: the only command the remote page may call.
//!
//! It mints (or reuses) the in-memory `desktop_key`, opens the system browser at the hosted start
//! URL with that key, and hands the key back to the page. Google refuses OAuth inside embedded
//! webviews, so the browser is the only place this can happen.
//!
//! What the page does next is 06-03's: it POSTs `{code, desktop_key}` to `/auth/desktop/exchange`,
//! a same-origin request that sends `Origin: https://vtradingview.com` — exactly what the CSRF
//! guard requires (D-010) — and puts `Set-Cookie` in the webview's own jar for free.

use std::time::Instant;

use tauri::{AppHandle, State};

use crate::config;
use crate::state::DesktopState;

#[tauri::command]
pub fn start_sign_in(app: AppHandle, st: State<'_, DesktopState>) -> Result<String, String> {
    let (key, reopen) = st.begin(Instant::now());

    if reopen {
        // The launch goes on its own thread. Measured on this box: a synchronous
        // `open_url` blocks for 84 s before returning `os error 1223` (ERROR_CANCELLED), and
        // Tauri runs a sync command on the main thread, so the whole window froze while the
        // shell looked for a browser. The page does not need this call's outcome to make
        // progress: its own 60 s overlay timeout is the failure path when no code ever arrives.
        let url = config::start_url(&key);
        std::thread::spawn(move || {
            if let Err(e) = tauri_plugin_opener::open_url(url, None::<&str>) {
                tracing::warn!(target: "desktop", "could not open the system browser: {e}");
            } else {
                tracing::info!(target: "desktop", "sign-in page handed to the system browser");
            }
        });
        // Length only: never the key, never a prefix of it.
        tracing::info!(target: "desktop", "sign-in started (key {} chars)", key.len());
    } else {
        tracing::debug!(target: "desktop", "sign-in already in flight; reused the key");
    }

    let _ = app;
    Ok(key)
}
