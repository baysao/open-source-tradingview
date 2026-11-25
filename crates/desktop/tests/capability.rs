//! The capability file is the phase's security boundary, so it gets a test that fails when the
//! boundary widens — not a comment asking future readers to be careful.
//!
//! It asserts the *manifest*, not a live invoke (a live invoke needs a running app and a GUI, which
//! CI does not have). The live half is the agent's window pass recorded in 06-01-SUMMARY.md.

use std::path::PathBuf;

fn capability() -> serde_json::Value {
    // CARGO_MANIFEST_DIR is the crate root, so this works from any cwd.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities/hub.json");
    let raw = std::fs::read_to_string(&path).expect("capabilities/hub.json is readable");
    serde_json::from_str(&raw).expect("capabilities/hub.json is valid JSON")
}

#[test]
fn the_remote_page_gets_exactly_one_permission() {
    let perms: Vec<String> = capability()["permissions"]
        .as_array()
        .expect("permissions is an array")
        .iter()
        .map(|p| p.as_str().expect("a permission is a string").to_string())
        .collect();

    assert_eq!(
        perms,
        vec!["allow-start-sign-in".to_string()],
        "the remote origin gets exactly one permission; a superset is the defect this test exists for"
    );
}

#[test]
fn no_plugin_or_core_permission_reaches_the_remote_origin() {
    // Belt and braces with the exact-list assertion above: if someone replaces it with a loop, this
    // still names the two families that must never appear.
    for p in capability()["permissions"]
        .as_array()
        .expect("permissions is an array")
    {
        let p = p.as_str().expect("a permission is a string");
        assert!(
            !p.starts_with("plugin:") && !p.starts_with("core:"),
            "{p} would hand the untrusted page a plugin or core surface"
        );
    }
}

#[test]
fn the_grant_is_remote_scoped_to_the_hosted_origin_only() {
    let cap = capability();
    let urls: Vec<String> = cap["remote"]["urls"]
        .as_array()
        .expect("remote.urls is an array")
        .iter()
        .map(|u| u.as_str().expect("a url is a string").to_string())
        .collect();
    assert_eq!(urls, vec!["https://vtradingview.com/*".to_string()]);
}

#[test]
fn the_grant_does_not_apply_to_local_urls() {
    // Mutation to prove this bites: delete `"local": false` and this fails. Without it the
    // capability also applies to app-local URLs, which nothing needs and which is one more surface.
    assert_eq!(
        capability()["local"],
        serde_json::Value::Bool(false),
        "the capability must not apply to local URLs"
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
