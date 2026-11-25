//! The tray: Show and Quit. The only way to exit the app, since closing the window hides it.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub const TRAY_ID: &str = "main";
const ITEM_SHOW: &str = "show";
const ITEM_QUIT: &str = "quit";

pub fn build(app: &AppHandle) -> tauri::Result<TrayIcon> {
    let show = MenuItem::with_id(app, ITEM_SHOW, "Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, ITEM_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    // include_image! decodes at compile time (tauri::include_image is the supported path; the
    // from_bytes constructor is behind a feature this crate does not enable).
    let icon = tauri::include_image!("icons/32x32.png");

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("TradeX")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            ITEM_SHOW => show_window(app),
            ITEM_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // A left click raises the window, the behaviour every tray app on this platform has.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)
}

fn show_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(crate::window::LABEL) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}
