//! The navigation guard: the window stays on the hosted origin and nothing else.
//!
//! This is `validate-origins` rule 1 applied where the privilege actually lives: "verify exact
//! origins for privileged actions". Its "Prefix and substring spoofing" case is not hypothetical
//! here — `https://vtradingview.com.evil.com` starts with `https://vtradingview.com`, so a prefix
//! check would let an attacker's page load inside a window that can call `start_sign_in`.

/// True only for the hosted origin: scheme, host and port all compared, never a prefix.
pub fn is_allowed(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("vtradingview.com")
        && url.port_or_known_default() == Some(443)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(s: &str) -> bool {
        is_allowed(&url::Url::parse(s).expect("test URL parses"))
    }

    #[test]
    fn the_hosted_pages_are_allowed() {
        assert!(allowed("https://vtradingview.com/hub.html"));
        assert!(allowed("https://vtradingview.com/"));
        assert!(allowed("https://vtradingview.com/hub.html?signin=failed#x"));
        // An explicit 443 is the same origin.
        assert!(allowed("https://vtradingview.com:443/hub.html"));
    }

    #[test]
    fn a_prefix_spoofed_host_is_refused() {
        // The whole reason the guard compares the host instead of the string.
        assert!(!allowed("https://vtradingview.com.evil.com/hub.html"));
        assert!(!allowed("https://evil.com/vtradingview.com"));
        assert!(!allowed("https://evil.com/?r=https://vtradingview.com"));
        assert!(!allowed("https://vtradingview.com@evil.com/hub.html"));
    }

    #[test]
    fn a_downgrade_to_plain_http_is_refused() {
        assert!(!allowed("http://vtradingview.com/hub.html"));
    }

    #[test]
    fn a_non_default_port_is_refused() {
        assert!(!allowed("https://vtradingview.com:8443/hub.html"));
    }

    #[test]
    fn everything_off_origin_is_refused() {
        // Google's own sign-in page must open in the system browser, not in the app window.
        assert!(!allowed("https://accounts.google.com/o/oauth2/auth"));
        assert!(!allowed("data:text/html,<p>x</p>"));
        assert!(!allowed("file:///etc/passwd"));
        assert!(!allowed("javascript:alert(1)"));
    }
}
