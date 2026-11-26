//! `pair`: hand the host's bridge to the owner's hub account.
//!
//! One request, one shape (09-01's contract):
//!
//! ```text
//! POST {hub}/api/me/mt5/pair   body {code, label, url, token}
//!   201 {id, label, url, symbol_count, connected, revoke_code}
//!   401 {"error":"code unknown or expired"}
//!   422 {"error":"<exact cause>"}
//! ```
//!
//! Three rules this module exists to keep:
//!
//! 1. **HTTPS only, checked before any request.** A `http://` hub URL is refused locally, so a
//!    bridge token never crosses a plaintext link by mistake.
//! 2. **No cookie, no Origin.** That is the real call shape of a machine with no browser session
//!    (D-037). This client sends neither, and a test asserts it.
//! 3. **The revoke code never reaches a log.** It arrives once; it is kept in a [`Secret`].

use url::Url;

use crate::http::{Http, HttpError, PAIR_TIMEOUT};
use crate::secret::Secret;

use super::Failure;

/// The hub path pair calls. Absolute against the hub's origin.
pub const PAIR_PATH: &str = "/api/me/mt5/pair";

#[derive(Clone, Debug)]
pub struct PairedBridge {
    pub id: String,
    pub label: String,
    pub url: String,
    pub symbol_count: u64,
    pub connected: bool,
    /// Shown once at pairing. Never logged (its `Debug` is a length).
    pub revoke_code: Secret,
}

#[derive(Clone, Debug)]
pub struct PairOutcome {
    pub bridge: PairedBridge,
}

/// Refuses anything but an https origin. Also refuses a URL with a path, query or userinfo: the
/// body carries the *bridge* URL, so the hub URL must be a bare origin and a typo here would
/// silently post a token to the wrong place.
pub fn hub_origin(raw: &str) -> Result<String, Failure> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Failure::Input("the hub URL is empty".to_string()));
    }
    let parsed = Url::parse(trimmed)
        .map_err(|e| Failure::Input(format!("the hub URL does not parse: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(Failure::Input(format!(
            "the hub URL must be https://, not {0}:// (the bridge token would cross in clear text)",
            parsed.scheme()
        )));
    }
    if parsed.host_str().is_none() {
        return Err(Failure::Input("the hub URL has no host".to_string()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Failure::Input(
            "the hub URL must not carry credentials".to_string(),
        ));
    }
    if parsed.path() != "/" && !parsed.path().is_empty() {
        return Err(Failure::Input(format!(
            "the hub URL must be an origin with no path, got {}",
            parsed.path()
        )));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(Failure::Input(
            "the hub URL must not carry a query or fragment".to_string(),
        ));
    }
    Ok(format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().expect("checked above")
    ) + &parsed.port().map(|p| format!(":{p}")).unwrap_or_default())
}

/// The pairing code as the hub mints it: ten digits, displayed grouped by five.
///
/// Accepted with spaces or without, because the Settings dialog groups them for reading.
pub fn normalize_code(raw: &str) -> Result<String, Failure> {
    let digits: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != 10 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(Failure::Input(format!(
            "the pairing code is ten digits, got {} characters",
            digits.len()
        )));
    }
    Ok(digits)
}

/// Builds the request body. Kept separate so a test can assert the exact wire shape.
pub fn pair_body(code: &str, label: &str, bridge_url: &str, token: &Secret) -> String {
    serde_json::json!({
        "code": code,
        "label": label,
        "url": bridge_url,
        "token": token.expose(),
    })
    .to_string()
}

/// `pair(hub_url, bridge_url, token, code)`.
///
/// `label` is the host's own name for the bridge; the caller supplies it (the UI defaults to the
/// computer name).
pub fn pair<H: Http>(
    http: &H,
    hub_url: &str,
    label: &str,
    bridge_url: &str,
    token: &Secret,
    code: &str,
) -> Result<PairOutcome, Failure> {
    let origin = hub_origin(hub_url)?;
    let code = normalize_code(code)?;
    let bridge = bridge_url.trim();
    if bridge.is_empty() {
        return Err(Failure::Input("the bridge URL is empty".to_string()));
    }
    let label = label.trim();
    if label.is_empty() {
        return Err(Failure::Input("the bridge label is empty".to_string()));
    }
    if token.is_empty() {
        return Err(Failure::Input("the bridge token is empty".to_string()));
    }

    let body = pair_body(&code, label, bridge, token);
    let url = format!("{origin}{PAIR_PATH}");
    let resp = http
        .post_json(&url, &body, PAIR_TIMEOUT)
        .map_err(Failure::from_http)?;

    match resp.status {
        201 => {
            let v: serde_json::Value =
                serde_json::from_str(&resp.body).map_err(|e| Failure::Hub {
                    status: 201,
                    cause: format!("the hub's 201 body does not parse: {e}"),
                })?;
            let field = |k: &str| -> Result<String, Failure> {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| Failure::Hub {
                        status: 201,
                        cause: format!("the hub's 201 body has no {k}"),
                    })
            };
            Ok(PairOutcome {
                bridge: PairedBridge {
                    id: field("id")?,
                    label: v
                        .get("label")
                        .and_then(|x| x.as_str())
                        .unwrap_or(label)
                        .to_string(),
                    url: v
                        .get("url")
                        .and_then(|x| x.as_str())
                        .unwrap_or(bridge)
                        .to_string(),
                    symbol_count: v
                        .get("symbol_count")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0),
                    connected: v
                        .get("connected")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false),
                    revoke_code: Secret::new(field("revoke_code").map_err(|_| Failure::Hub {
                        status: 201,
                        cause:
                            "the hub's 201 body has no revoke_code; the bridge cannot be unregistered from here"
                                .to_string(),
                    })?),
                },
            })
        }
        status => Err(super::hub_cause(status, &resp.body)),
    }
}

/// Unreachable-transport shortcut used by the app: a failed connect is a [`Failure::Transport`].
pub fn transport(e: HttpError) -> Failure {
    Failure::Transport(e.cause())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Fake, HttpResponse};

    fn token() -> Secret {
        Secret::new("bridge-token-abcd".to_string())
    }

    #[test]
    fn an_http_hub_url_is_refused_before_any_request() {
        let http = Fake::default();
        let err = pair(
            &http,
            "http://vtradingview.com",
            "host",
            "https://b.example",
            &token(),
            "1234567890",
        )
        .expect_err("http refused");
        assert!(err.cause().contains("must be https://"), "{}", err.cause());
        assert_eq!(
            http.total_calls(),
            0,
            "no request may leave for a plaintext hub"
        );
    }

    #[test]
    fn the_request_is_the_pair_contract_exactly_and_sends_no_cookie_or_origin() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 201,
            body: r#"{"id":"b1","label":"host","url":"https://b.example","symbol_count":358,"connected":true,"revoke_code":"rv-1"}"#.to_string(),
        })]);
        let out = pair(
            &http,
            "https://vtradingview.com",
            "host",
            "https://b.example",
            &token(),
            "12345 67890",
        )
        .expect("201 parses");
        assert_eq!(out.bridge.id, "b1");
        assert_eq!(out.bridge.symbol_count, 358);
        assert!(out.bridge.connected);
        assert_eq!(out.bridge.revoke_code.expose(), "rv-1");

        let posts = http.post_calls();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].0, "https://vtradingview.com/api/me/mt5/pair");
        let body: serde_json::Value = serde_json::from_str(&posts[0].1).expect("json body");
        assert_eq!(body["code"], "1234567890", "the grouped code is normalised");
        assert_eq!(body["label"], "host");
        assert_eq!(body["url"], "https://b.example");
        assert_eq!(body["token"], "bridge-token-abcd");
        assert_eq!(
            body.as_object().expect("object").len(),
            4,
            "exactly the four contract keys; a fifth would be a different contract"
        );
        // D-037: this path takes no cookie and no Origin, and the client sends neither by
        // construction — the seam has no header for them.
        assert_eq!(http.get_calls().len(), 0);
    }

    #[test]
    fn a_401_surfaces_the_hubs_verbatim_text_and_says_mint_a_new_code() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 401,
            body: r#"{"error":"code unknown or expired"}"#.to_string(),
        })]);
        let err = pair(
            &http,
            "https://h.example",
            "host",
            "https://b.example",
            &token(),
            "1234567890",
        )
        .expect_err("401");
        assert_eq!(err.cause(), "code unknown or expired");
        assert!(err.next_action().unwrap_or("").contains("fresh one"));
    }

    #[test]
    fn a_422_surfaces_the_guards_exact_cause() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 422,
            body: r#"{"error":"url refused: private address"}"#.to_string(),
        })]);
        let err = pair(
            &http,
            "https://h.example",
            "host",
            "https://10.0.0.1",
            &token(),
            "1234567890",
        )
        .expect_err("422");
        assert_eq!(err.cause(), "url refused: private address");
        assert!(err.next_action().unwrap_or("").contains("new code"));
    }

    #[test]
    fn a_transport_failure_names_the_transport() {
        let http = Fake::with_posts(vec![Err(HttpError::Transport("dns error".to_string()))]);
        let err = pair(
            &http,
            "https://h.example",
            "host",
            "https://b.example",
            &token(),
            "1234567890",
        )
        .expect_err("transport");
        assert_eq!(err.cause(), "dns error");
        assert!(err.next_action().unwrap_or("").contains("reach the hub"));
    }

    #[test]
    fn a_201_without_revoke_code_is_a_hard_failure() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 201,
            body: r#"{"id":"b1","label":"h","url":"u","symbol_count":1,"connected":true}"#
                .to_string(),
        })]);
        let err = pair(
            &http,
            "https://h.example",
            "host",
            "https://b.example",
            &token(),
            "1234567890",
        )
        .expect_err("no revoke_code");
        assert!(err.cause().contains("revoke_code"), "{}", err.cause());
    }

    #[test]
    fn a_bad_code_is_refused_before_the_request() {
        for bad in ["12345", "123456789a", ""] {
            let http = Fake::default();
            let err = pair(
                &http,
                "https://h.example",
                "host",
                "https://b.example",
                &token(),
                bad,
            )
            .expect_err("bad code");
            assert!(err.cause().contains("ten digits"), "{bad}: {}", err.cause());
            assert_eq!(http.total_calls(), 0);
        }
    }

    #[test]
    fn a_hub_url_with_a_path_or_credentials_is_refused() {
        for bad in [
            "https://h.example/api",
            "https://user:pw@h.example",
            "https://h.example?x=1",
        ] {
            let err = hub_origin(bad).expect_err("refused");
            assert!(matches!(err, Failure::Input(_)), "{bad}: {err}");
        }
        assert_eq!(
            hub_origin("https://h.example/").expect("bare origin ok"),
            "https://h.example"
        );
        assert_eq!(
            hub_origin("https://h.example:8443").expect("port ok"),
            "https://h.example:8443"
        );
    }

    #[test]
    fn the_wire_body_holds_the_token_it_was_given() {
        let body = pair_body("1234567890", "host", "https://b", &token());
        assert!(body.contains("bridge-token-abcd"));
    }

    #[test]
    fn the_revoke_code_does_not_print_through_the_outcome() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 201,
            body: r#"{"id":"b1","label":"h","url":"u","symbol_count":1,"connected":true,"revoke_code":"SECRET-REVOKE"}"#.to_string(),
        })]);
        let out = pair(
            &http,
            "https://h.example",
            "host",
            "https://b.example",
            &token(),
            "1234567890",
        )
        .expect("201");
        let printed = format!("{out:?}");
        assert!(!printed.contains("SECRET-REVOKE"), "{printed}");
        assert!(printed.contains("13 chars"), "{printed}");
    }
}
