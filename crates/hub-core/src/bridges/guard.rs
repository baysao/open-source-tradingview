//! The SSRF guard (07-ARCHITECTURE, spec `:354-359`): a registered bridge URL must be
//! `https://` with no user info on port 443 or 1024..=65535, and every address its host resolves
//! to must be public. The refused classes are loopback, private (RFC 1918), CGNAT (100.64/10),
//! link-local (v4 and v6), unique-local (fc00::/7), multicast, unspecified, and the IPv4-mapped
//! form of each. With auth off, loopback is allowed (D-001; the mechanics are D-033's env
//! bridge) and every other class stays refused. Plain `http://` is accepted with auth off only
//! when every resolved address is loopback: that is the local-mode bridge, never a remote one.
//! [`Vetted`] pins the vetted addresses so the client connects to exactly what was checked (a
//! second DNS answer cannot redirect the call).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Why the guard refused a URL or an address. The `Display` text is what the user sees (CUJ 7:
/// "address refused" with the specific reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a parseable URL, the scheme is not https (and the auth-off local exception does not
    /// apply), or the host is missing.
    NotHttps,
    /// The URL carries user info (`user:pass@host`).
    UserInfo,
    /// The port is not 443 and not in 1024..=65535.
    BadPort,
    /// The host did not resolve, or resolved to nothing.
    Dns,
    /// An address class the guard refuses; the string is [`refused_reason`]'s own words.
    Host(&'static str),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::NotHttps => write!(f, "the bridge url must be https"),
            Refusal::UserInfo => write!(f, "the bridge url must not carry user info"),
            Refusal::BadPort => write!(f, "the bridge port must be 443 or 1024 to 65535"),
            Refusal::Dns => write!(f, "the bridge host did not resolve"),
            Refusal::Host(reason) => write!(f, "the bridge address was refused: {reason}"),
        }
    }
}

/// A vetted bridge endpoint: the normalized URL (fragment stripped), its host as the URL writes
/// it, its port, and the addresses a call may connect to (all vetted).
#[derive(Debug, Clone, PartialEq)]
pub struct Vetted {
    pub url: reqwest::Url,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<IpAddr>,
}

/// Loopback in any form: `127.0.0.0/8`, `::1`, and the IPv4-mapped loopback (`::ffff:127.x`).
fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|m| m.is_loopback()),
    }
}

/// Why an address is refused, or `None` when it may be dialed. Pure and table-driven; every
/// class has a unit case, including the IPv4-mapped forms. With `auth_enabled = false`, loopback
/// (v4, v6 and mapped) is allowed and nothing else changes (D-001/D-033).
pub fn refused_reason(ip: IpAddr, auth_enabled: bool) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => refused_v4(v4, auth_enabled),
        IpAddr::V6(v6) => refused_v6(v6, auth_enabled),
    }
}

fn refused_v4(ip: Ipv4Addr, auth_enabled: bool) -> Option<&'static str> {
    if ip.is_loopback() {
        return auth_enabled.then_some("loopback address");
    }
    if ip.is_private() {
        return Some("private (RFC 1918) address");
    }
    let o = ip.octets();
    // CGNAT 100.64/10 (`is_shared` is unstable; the two-byte check is the range).
    if o[0] == 100 && o[1] >= 64 && o[1] <= 127 {
        return Some("CGNAT address");
    }
    if ip.is_link_local() {
        return Some("link-local address");
    }
    if ip.is_multicast() {
        return Some("multicast address");
    }
    if ip.is_unspecified() {
        return Some("unspecified address");
    }
    if ip.is_broadcast() {
        return Some("broadcast address");
    }
    None
}

fn refused_v6(ip: Ipv6Addr, auth_enabled: bool) -> Option<&'static str> {
    // The IPv4-mapped form takes the same verdicts as the address it maps (spec `:357`).
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return refused_v4(mapped, auth_enabled);
    }
    if ip.is_loopback() {
        return auth_enabled.then_some("loopback address");
    }
    let o = ip.octets();
    // Unique-local fc00::/7 (`is_unique_local` is unstable; the mask is the range).
    if o[0] & 0xFE == 0xFC {
        return Some("unique-local address");
    }
    // Link-local fe80::/10.
    if o[0] == 0xFE && o[1] & 0xC0 == 0x80 {
        return Some("link-local address");
    }
    if ip.is_multicast() {
        return Some("multicast address");
    }
    if ip.is_unspecified() {
        return Some("unspecified address");
    }
    None
}

/// The refusal for a numeric host (`127.0.0.1`, `[::1]`), or `None` when the host is a domain
/// name (or the text is not an address at all, which DNS will decide).
fn numeric_host_refusal(host: Option<&str>, auth_enabled: bool) -> Option<&'static str> {
    let text = host?;
    let bare = text
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(text);
    let ip: IpAddr = bare.parse().ok()?;
    refused_reason(ip, auth_enabled)
}

/// The syntax half of the vet: https only (except the auth-off local bridge, D-033), no user
/// info, port 443 or 1024..=65535, fragment stripped, numeric hosts through the refusal table.
/// No DNS here; [`vet_url`] adds the resolution and the address rules.
pub fn vet_syntax(raw: &str, auth_enabled: bool) -> Result<reqwest::Url, Refusal> {
    let mut url = reqwest::Url::parse(raw).map_err(|_| Refusal::NotHttps)?;
    let https = url.scheme() == "https";
    if !https && auth_enabled {
        return Err(Refusal::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Refusal::UserInfo);
    }
    match url.port_or_known_default() {
        Some(443) => {}
        Some(p) if (1024..=65535).contains(&p) => {}
        _ => return Err(Refusal::BadPort),
    }
    if let Some(reason) = numeric_host_refusal(url.host_str(), auth_enabled) {
        return Err(Refusal::Host(reason));
    }
    url.set_fragment(None);
    Ok(url)
}

/// The full vet: syntax, then resolve the host and refuse if any resolved address is in a
/// refused class (spec: "refuse if any resolved address is ..."). Plain http with auth off is
/// the local bridge: every resolved address must then be loopback (D-033). `Vetted.addrs`
/// carries exactly the resolved set, so the client pins to what was checked.
pub async fn vet_url(raw: &str, auth_enabled: bool) -> Result<Vetted, Refusal> {
    let url = vet_syntax(raw, auth_enabled)?;
    let port = url.port_or_known_default().unwrap_or(443);
    let host = url.host_str().ok_or(Refusal::NotHttps)?.to_string();
    let mut addrs = Vec::new();
    for sa in tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| Refusal::Dns)?
    {
        let ip = sa.ip();
        if let Some(reason) = refused_reason(ip, auth_enabled) {
            return Err(Refusal::Host(reason));
        }
        if url.scheme() != "https" && !is_loopback(ip) {
            return Err(Refusal::NotHttps);
        }
        addrs.push(ip);
    }
    if addrs.is_empty() {
        return Err(Refusal::Dns);
    }
    Ok(Vetted {
        url,
        host,
        port,
        addrs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(raw: &str) -> Refusal {
        vet_syntax(raw, true).expect_err("expected a refusal")
    }

    #[test]
    fn every_refused_class_refuses_with_auth_on() {
        let cases: &[(&str, &'static str)] = &[
            ("https://127.0.0.1:8443", "loopback address"),
            ("https://[::1]:8443", "loopback address"),
            ("https://10.1.2.3:8443", "private (RFC 1918) address"),
            ("https://172.16.0.9:8443", "private (RFC 1918) address"),
            ("https://192.168.5.5:8443", "private (RFC 1918) address"),
            ("https://100.64.0.1:8443", "CGNAT address"),
            ("https://100.127.255.254:8443", "CGNAT address"),
            ("https://169.254.169.254:8443", "link-local address"),
            ("https://[fe80::1]:8443", "link-local address"),
            ("https://[fc00::1]:8443", "unique-local address"),
            ("https://[fd12:3456::1]:8443", "unique-local address"),
            ("https://[ff02::1]:8443", "multicast address"),
            ("https://[::]:8443", "unspecified address"),
            // The IPv4-mapped forms take the v4 verdict.
            (
                "https://[::ffff:10.0.0.1]:8443",
                "private (RFC 1918) address",
            ),
            ("https://[::ffff:127.0.0.1]:8443", "loopback address"),
            (
                "https://[::ffff:169.254.169.254]:8443",
                "link-local address",
            ),
        ];
        for (raw, reason) in cases {
            assert_eq!(refused(raw), Refusal::Host(reason), "{raw}");
        }
    }

    #[test]
    fn auth_off_only_lifts_loopback() {
        assert!(vet_syntax("https://127.0.0.1:8443", false).is_ok());
        assert!(vet_syntax("https://[::1]:8443", false).is_ok());
        assert!(vet_syntax("https://[::ffff:127.0.0.1]:8443", false).is_ok());
        assert!(vet_syntax("http://127.0.0.1:8797", false).is_ok());
        // Every other class stays refused, in either scheme.
        assert_eq!(
            vet_syntax("https://10.1.2.3:8443", false),
            Err(Refusal::Host("private (RFC 1918) address"))
        );
        assert_eq!(
            vet_syntax("http://10.1.2.3:8797", false),
            Err(Refusal::Host("private (RFC 1918) address"))
        );
    }

    #[test]
    fn syntax_rules() {
        assert_eq!(refused("http://bridge.example.com:8443"), Refusal::NotHttps);
        assert_eq!(refused("not a url"), Refusal::NotHttps);
        assert_eq!(refused("https://"), Refusal::NotHttps);
        assert_eq!(
            refused("https://user:pass@bridge.example.com"),
            Refusal::UserInfo
        );
        assert_eq!(refused("https://bridge.example.com:80"), Refusal::BadPort);
        assert_eq!(refused("https://bridge.example.com:1023"), Refusal::BadPort);
        // 443 by name and by number, and the whole custom range.
        assert!(vet_syntax("https://bridge.example.com", true).is_ok());
        assert!(vet_syntax("https://bridge.example.com:443", true).is_ok());
        assert!(vet_syntax("https://bridge.example.com:1024", true).is_ok());
        assert!(vet_syntax("https://bridge.example.com:65535", true).is_ok());
        // Public addresses pass.
        assert!(vet_syntax("https://8.8.8.8:8443", true).is_ok());
        assert!(vet_syntax("https://[2606:4700::1111]:8443", true).is_ok());
    }

    #[tokio::test]
    async fn dns_refusal_and_pin() {
        // A name that does not resolve.
        assert_eq!(
            vet_url("https://no-such-host-07-01.invalid", true).await,
            Err(Refusal::Dns)
        );
        // A numeric host resolves to itself and is vetted without DNS.
        let v = vet_url("https://127.0.0.1:8443", false)
            .await
            .expect("loopback, auth off");
        assert_eq!(v.addrs, vec![IpAddr::from([127, 0, 0, 1])]);
        assert_eq!(v.port, 8443);
        // Fragments are stripped from the normalized URL.
        let v = vet_url("https://127.0.0.1:8443/base#frag", false)
            .await
            .unwrap();
        assert!(!v.url.as_str().contains('#'));
        // Plain http with auth off must resolve to loopback only (D-033); a public host over
        // plain http is refused even with auth off.
        assert!(vet_url("http://127.0.0.1:8797", false).await.is_ok());
        assert_eq!(
            vet_url("http://8.8.8.8:8797", false).await,
            Err(Refusal::NotHttps)
        );
    }
}
