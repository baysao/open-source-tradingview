//! `revoke`: unregister exactly this bridge.
//!
//! ```text
//! POST {hub}/api/me/mt5/paired/{id}/revoke   body {revoke_code}
//!   204 -> deleted
//!   404 -> no bridge carries that id (already revoked, or never paired)
//!   401 {"error":"revoke code unknown or already used"}
//! ```
//!
//! The secret is the authority: no session, no owner check (09-01's B9). So the app must never
//! send it anywhere but the hub origin it paired with, and must never log it.

use url::Url;

use crate::http::{Http, REVOKE_TIMEOUT};
use crate::secret::Secret;

use super::pair::hub_origin;
use super::Failure;

/// The revoke path, with the id as the hub's own route expects (a UUID in path position).
pub fn revoke_path(id: &str) -> Result<String, Failure> {
    // The id comes from the hub's own 201 and is a UUID. Percent-encoding anything else would
    // paper over a malformed id, so it is checked and refused instead.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(Failure::Input(format!(
            "the bridge id is not a plain id: {id}"
        )));
    }
    Ok(format!("/api/me/mt5/paired/{id}/revoke"))
}

/// Deletes the bridge that `revoke_code` was issued for. 204 is success; 404 means it is already
/// gone, which the UI reports as success-with-a-note rather than an error.
pub fn revoke<H: Http>(
    http: &H,
    hub_url: &str,
    id: &str,
    revoke_code: &Secret,
) -> Result<(), Failure> {
    let origin = hub_origin(hub_url)?;
    let path = revoke_path(id)?;
    if revoke_code.is_empty() {
        return Err(Failure::Input(
            "no revoke code is held for this bridge: it can only be unregistered in the hub's Settings"
                .to_string(),
        ));
    }
    let url = format!("{origin}{path}");
    // A URL that cannot be parsed is a local mistake, not a hub answer.
    Url::parse(&url).map_err(|e| Failure::Input(format!("the revoke URL does not parse: {e}")))?;
    let body = serde_json::json!({ "revoke_code": revoke_code.expose() }).to_string();
    let resp = http
        .post_json(&url, &body, REVOKE_TIMEOUT)
        .map_err(Failure::from_http)?;
    match resp.status {
        204 | 200 => Ok(()),
        404 => Err(Failure::Hub {
            status: 404,
            cause: "this bridge is no longer registered".to_string(),
        }),
        status => Err(super::hub_cause(status, &resp.body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Fake, HttpResponse};

    fn secret() -> Secret {
        Secret::new("rev-abc".to_string())
    }

    #[test]
    fn a_204_is_success_and_the_body_is_the_revoke_code_only() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 204,
            body: String::new(),
        })]);
        revoke(
            &http,
            "https://h.example",
            "11111111-2222-3333-4444-555555555555",
            &secret(),
        )
        .expect("204");
        let posts = http.post_calls();
        assert_eq!(
            posts[0].0,
            "https://h.example/api/me/mt5/paired/11111111-2222-3333-4444-555555555555/revoke"
        );
        assert_eq!(posts[0].1, r#"{"revoke_code":"rev-abc"}"#);
    }

    #[test]
    fn a_404_says_the_bridge_is_already_gone() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 404,
            body: r#"{"error":"not found"}"#.to_string(),
        })]);
        let err = revoke(&http, "https://h.example", "b1", &secret()).expect_err("404");
        assert!(err.cause().contains("no longer registered"));
    }

    #[test]
    fn a_401_surfaces_the_hub_text() {
        let http = Fake::with_posts(vec![Ok(HttpResponse {
            status: 401,
            body: r#"{"error":"revoke code unknown or already used"}"#.to_string(),
        })]);
        let err = revoke(&http, "https://h.example", "b1", &secret()).expect_err("401");
        assert_eq!(err.cause(), "revoke code unknown or already used");
    }

    #[test]
    fn an_id_that_is_not_plain_is_refused_before_the_request() {
        for bad in ["", "../admin", "a/b", "a b"] {
            let http = Fake::default();
            let err = revoke(&http, "https://h.example", bad, &secret()).expect_err("bad id");
            assert_eq!(http.total_calls(), 0, "{bad}");
            assert!(matches!(err, Failure::Input(_)), "{bad}: {err}");
        }
    }

    #[test]
    fn an_empty_revoke_code_says_where_it_could_have_come_from() {
        let http = Fake::default();
        let err = revoke(
            &http,
            "https://h.example",
            "b1",
            &Secret::new(String::new()),
        )
        .expect_err("empty");
        assert!(err.cause().contains("Settings"), "{}", err.cause());
        assert_eq!(http.total_calls(), 0);
    }
}
