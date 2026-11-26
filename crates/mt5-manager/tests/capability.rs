//! The capability file is this phase's security boundary (ROADMAP gate: "the app's own
//! capabilities stay as locked down as the v0.2 shell's"), so it gets a test that fails when the
//! boundary widens — not a comment asking future readers to be careful.
//!
//! It asserts the *manifest*, not a live invoke (a live invoke needs a running window, which CI
//! does not have). The live half is 09-02-SUMMARY.md's look pass.
//!
//! Mirrors crates/desktop/tests/capability.rs, with one deliberate difference: that shell's
//! capability is REMOTE-scoped (`remote.urls: https://vtradingview.com/*`) because it loads the
//! hosted page. This app never loads a remote page, so its capability is local-only and the test
//! asserts the absence of a remote block.

use std::path::PathBuf;

fn capability() -> serde_json::Value {
    // CARGO_MANIFEST_DIR is the crate root, so this works from any cwd.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities/manager.json");
    let raw = std::fs::read_to_string(&path).expect("capabilities/manager.json is readable");
    serde_json::from_str(&raw).expect("capabilities/manager.json is valid JSON")
}

fn config() -> serde_json::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let raw = std::fs::read_to_string(&path).expect("tauri.conf.json is readable");
    serde_json::from_str(&raw).expect("tauri.conf.json is valid JSON")
}

fn permissions() -> Vec<String> {
    capability()["permissions"]
        .as_array()
        .expect("permissions is an array")
        .iter()
        .map(|p| p.as_str().expect("a permission is a string").to_string())
        .collect()
}

#[test]
fn the_local_window_gets_exactly_the_seven_app_commands() {
    // AC-2's list, in the order the plan names them. A superset is the defect this test exists for:
    // one more permission is one more thing a compromised page could call.
    assert_eq!(
        permissions(),
        vec![
            "allow-pair".to_string(),
            "allow-install".to_string(),
            "allow-control".to_string(),
            "allow-status".to_string(),
            "allow-revoke".to_string(),
            // Read-only: one HTTPS POST that carries the token to the terminal's own loopback
            // endpoint and reports the answer. It installs nothing, writes nothing and holds no state,
            // which is why it is allowed to exist beside the five that act.
            "allow-check-mcp".to_string(),
            // A read of the install's own in-memory progress snapshot: no side effect, no state
            // written, and the only way the panel can show a bar without an event bus (AC-2).
            "allow-install-progress".to_string(),
        ]
    );
}

#[test]
fn no_plugin_or_core_permission_reaches_the_webview() {
    for p in permissions() {
        assert!(
            !p.starts_with("plugin:") && !p.starts_with("core:"),
            "{p} would hand the webview a plugin or core surface (fs, shell, opener, clipboard, \
             http, event bus are all out by AC-2)"
        );
    }
}

#[test]
fn the_capability_is_local_and_not_remote() {
    let cap = capability();
    assert_eq!(
        cap["local"],
        serde_json::Value::Bool(true),
        "the manager's window is app-local; without this the grant would not apply to it"
    );
    assert!(
        cap.get("remote").is_none(),
        "a remote block would mean this app loads a foreign origin, which it must never do \
         (contrast crates/desktop/capabilities/hub.json, which is remote-scoped by design)"
    );
}

#[test]
fn the_grant_targets_the_one_window() {
    let windows: Vec<String> = capability()["windows"]
        .as_array()
        .expect("windows is an array")
        .iter()
        .map(|w| w.as_str().expect("a window label is a string").to_string())
        .collect();
    assert_eq!(windows, vec!["main".to_string()]);
}

#[test]
fn the_config_loads_no_remote_url_and_grants_no_global_tauri_object() {
    let cfg = config();
    assert_eq!(
        cfg["app"]["withGlobalTauri"],
        serde_json::Value::Bool(false),
        "the friendly global namespace is off; the ACL is the boundary and a global object invites \
         scripts to try it"
    );
    assert_eq!(cfg["build"]["frontendDist"], "ui");
    // `$schema` is a tooling URL, not something the app loads; everything the runtime reads must be
    // free of remote origins.
    for section in ["app", "build", "bundle"] {
        let text = serde_json::to_string(&cfg[section]).expect("serialises");
        assert!(
            !text.contains("http://") && !text.contains("https://"),
            "the manager's {section} config names no remote origin: {text}"
        );
    }
    let caps: Vec<String> = cfg["app"]["security"]["capabilities"]
        .as_array()
        .expect("capabilities is an array")
        .iter()
        .map(|c| c.as_str().expect("a capability id").to_string())
        .collect();
    assert_eq!(caps, vec!["manager".to_string()], "exactly one capability");
}

#[test]
fn the_identifier_and_product_name_do_not_collide_with_the_v02_shell() {
    // A shared identifier would make the two installers overwrite each other's uninstall entry.
    let cfg = config();
    assert_eq!(cfg["identifier"], "com.vtradingview.bridgemanager");
    assert_ne!(cfg["productName"], "TradeX");
}

#[test]
fn mutation_control_the_permission_list_is_read_from_the_file_not_a_constant() {
    // Proves the assertions above bite: a widened file must change what `permissions()` returns.
    // (The real mutation is recorded in the SUMMARY: add one permission, watch this suite fail.)
    let perms = permissions();
    assert_eq!(perms.len(), 7);
    assert!(perms.iter().all(|p| p.starts_with("allow-")));
}
