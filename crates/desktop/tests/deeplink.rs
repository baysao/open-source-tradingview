//! The `vtradingview://` hand-off, exercised without a window.
//!
//! `parse_code` is the only public entry a test needs; the delivery/queue behaviour is covered by
//! the unit tests in `src/deeplink.rs`, which is where the `Emitter` test double lives.

use desktop::deeplink::parse_code;

const GOOD: &str = "abcdefghij_klmnopqrstu";

/// Every shape the server can send, and the shapes an attacker might.
#[test]
fn the_parser_accepts_only_the_servers_shape() {
    let accepted = [
        format!("vtradingview://auth?code={GOOD}"),
        format!("vtradingview://auth?code={}", "a".repeat(43)),
    ];
    for url in accepted {
        assert!(parse_code(&url).is_some(), "{url} should be accepted");
    }

    let refused = [
        "vtradingview://auth?code=short".to_string(),
        format!("https://vtradingview.com/auth?code={GOOD}"),
        format!("vtradingview://other?code={GOOD}"),
        format!("vtradingview://auth?code={GOOD}&code={GOOD}"),
        "vtradingview://auth".to_string(),
        "vtradingview://auth?code=".to_string(),
        format!("vtradingview://auth?code={}", "a".repeat(129)),
        "vtradingview:///auth?code=abcdefghij_klmnopqrstu".to_string(),
        "javascript:alert(1)".to_string(),
        String::new(),
    ];
    for url in refused {
        assert!(parse_code(&url).is_none(), "{url} should be refused");
    }
}

/// The exact URL `crates/hub-core/src/routes/auth.rs:82` formats is the one the shell must accept.
/// This is the wire contract between the two crates, asserted on both sides: the server's own test
/// (`crates/hub-core/tests/auth.rs:411`) checks the prefix, this one checks the parse.
#[test]
fn the_servers_own_url_shape_round_trips() {
    let code = "XlkzUjMiL2oRwFGHMRuJpP_Q2TaGDF0cICZXEpMNALs"; // a real 43-char base64url key shape
    let url = format!("vtradingview://auth?code={code}");
    assert_eq!(parse_code(&url).as_deref(), Some(code));
}
