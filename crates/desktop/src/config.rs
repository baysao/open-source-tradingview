//! The hosted origin, the deep-link scheme, and the one command's URL.
//!
//! The spec's C22 proposed taking these from `hub-types`, and this crate does not: it is its own
//! workspace (D-030), and a second workspace cannot path-depend on a crate that uses
//! `edition.workspace = true` (`crates/hub-types/Cargo.toml:3`). Nothing is lost — the deployed
//! origin comes from `/etc/vtv/env`, so a compile-time constant could never detect a mismatch.

/// The only origin the window may navigate to.
pub const PUBLIC_ORIGIN: &str = "https://vtradingview.com";

/// The scheme the installed app registers, and the one `crates/hub-core/src/routes/auth.rs:82`
/// formats its redirect with.
pub const DEEP_LINK_SCHEME: &str = "vtradingview";

/// The page the window opens.
pub const HUB_PATH: &str = "/hub.html";

/// 32 random bytes, base64url-encoded without padding: 43 characters. The server accepts
/// 22..=128 (`crates/hub-core/src/auth/rules.rs` parse_client), and 43 is the same shape the
/// session id has (`crates/hub-core/src/auth/ids.rs:13`).
pub const KEY_BYTES: usize = 32;

/// The system-browser start URL for a desktop sign-in.
///
/// `client=desktop&dk=<key>` in that order: `crates/hub-core/src/routes/auth.rs:37` reads `client`
/// then `dk`, and `parse_client` refuses `client=desktop` with a missing or malformed `dk`.
/// Nothing is percent-encoded: the key is base64url, already URL-safe.
pub fn start_url(desktop_key: &str) -> String {
    format!("{PUBLIC_ORIGIN}/auth/google/start?client=desktop&dk={desktop_key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_url_names_the_desktop_client_and_carries_the_key() {
        let key = "a".repeat(43);
        let url = start_url(&key);
        assert_eq!(
            url,
            format!("https://vtradingview.com/auth/google/start?client=desktop&dk={key}")
        );
        // The order matters: the route reads `client` before `dk`.
        let client_at = url.find("client=desktop").expect("client param");
        let dk_at = url.find("dk=").expect("dk param");
        assert!(client_at < dk_at, "client must precede dk");
    }

    #[test]
    fn the_hub_page_is_one_path_segment_under_the_public_origin() {
        // static_files::resolve serves exactly one segment or vendor/<file> (L-035), so a nested
        // path would 404 and this constant must stay flat.
        assert_eq!(HUB_PATH, "/hub.html");
        assert!(!HUB_PATH[1..].contains('/'));
        assert_eq!(
            format!("{PUBLIC_ORIGIN}{HUB_PATH}"),
            "https://vtradingview.com/hub.html"
        );
    }

    #[test]
    fn key_bytes_encode_to_the_length_the_server_accepts() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let encoded = URL_SAFE_NO_PAD.encode([0u8; KEY_BYTES]);
        assert_eq!(encoded.len(), 43);
        assert!((22..=128).contains(&encoded.len()));
    }
}
