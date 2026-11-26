//! The thin builder. Everything the app does lives in the library (or in `run`).

#[cfg(feature = "gui")]
fn main() {
    mt5_manager::run();
}

/// Without tauri a binary cannot show a window, but the crate must still build and test (AC-1).
/// Saying so and exiting non-zero beats a silent no-op that looks like a working app.
#[cfg(not(feature = "gui"))]
fn main() {
    eprintln!("mt5-manager: built without the `gui` feature; rebuild with --features gui");
    std::process::exit(2);
}
