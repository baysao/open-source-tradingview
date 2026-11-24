//! The hosted auth handlers: One Tap, the button flow (`start`, `callback`), desktop exchange
//! and logout. Each is thin: parse, one `Auth` call, render (architecture decision 1). The gate
//! has already checked Origin, JSON and size.

use super::api_error;
use crate::auth::{AuthError, CallbackParams, Landing, SetCookie};
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

/// The sealed oauth ticket cookie (auth owns the format; the handler only reads the value).
const OAUTH_COOKIE: &str = "__Host-vtv_oauth";

/// `POST /auth/google/onetap` with `{"credential": "..."}`.
pub async fn onetap(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    #[derive(Deserialize)]
    struct OneTap {
        credential: String,
    }
    let Ok(b) = serde_json::from_slice::<OneTap>(&body) else {
        return bad_request();
    };
    match auth
        .sign_in_with_id_token(&b.credential, user_agent(&headers))
        .await
    {
        Ok(s) => signed_in(s),
        Err(e) => auth_error(e),
    }
}

/// `GET /auth/google/start?client=web|desktop&dk=...`: 302 to the Idp's authorization URL with
/// the sealed oauth cookie. An unknown `client` or a bad `dk` is 400.
pub async fn start(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match auth.begin_sign_in(
        q.get("client").map(|s| s.as_str()),
        q.get("dk").map(|s| s.as_str()),
    ) {
        Ok((url, cookies)) => {
            let mut resp = (StatusCode::FOUND, [("Location", url)]).into_response();
            resp = cookies.attach(resp);
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            resp
        }
        Err(e) => auth_error(e),
    }
}

/// `GET /auth/google/callback`: never fails; every refusal lands on `/hub.html?signin=failed`
/// with the oauth cookie cleared. A desktop client is redirected to the deep link with the code.
/// The sealed oauth ticket arrives in the Cookie header, not the query.
pub async fn callback(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let ticket = cookie_value(&headers, OAUTH_COOKIE);
    let ua = user_agent(&headers).map(|s| s.to_string());
    let p = CallbackParams {
        state: q.get("state").map(|s| s.as_str()),
        code: q.get("code").map(|s| s.as_str()),
        error: q.get("error").map(|s| s.as_str()),
        ticket: ticket.as_deref(),
    };
    match auth.finish_callback(p, ua.as_deref()).await {
        Landing::Web { cookies } => redirect("/hub.html", cookies),
        Landing::Desktop { code, cookies } => {
            let loc = format!("vtradingview://auth?code={code}");
            redirect(&loc, cookies)
        }
        Landing::Failed { cookies } => redirect("/hub.html?signin=failed", cookies),
    }
}

/// `POST /auth/desktop/exchange` with `{"code": "...", "desktop_key": "..."}`: burns the code and
/// mints a fresh session; the response carries the session cookie.
pub async fn desktop_exchange(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    #[derive(Deserialize)]
    struct Exchange {
        code: String,
        desktop_key: String,
    }
    let Ok(b) = serde_json::from_slice::<Exchange>(&body) else {
        return bad_request();
    };
    match auth
        .exchange_desktop_code(&b.code, &b.desktop_key, user_agent(&headers))
        .await
    {
        Ok(s) => signed_in(s),
        Err(e) => auth_error(e),
    }
}

/// `POST /auth/logout`: 204 with `Max-Age=0`; idempotent (signed out is still 204).
pub async fn logout(State(st): State<AppState>, Extension(viewer): Extension<Viewer>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match auth.sign_out(&viewer).await {
        Ok(cookies) => {
            let mut resp = StatusCode::NO_CONTENT.into_response();
            resp = cookies.attach(resp);
            resp
        }
        Err(e) => auth_error(e),
    }
}

// --- helpers -----------------------------------------------------------------------

use crate::auth::{SignedIn, Viewer};

/// The value of one cookie from the Cookie header.
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    for v in headers.get_all(header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for pair in s.split(';') {
            let pair = pair.trim();
            if let Some(rest) = pair.strip_prefix(name) {
                if let Some(value) = rest.strip_prefix('=') {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

fn signed_in(s: SignedIn) -> Response {
    let body = json!({"user": s.member.user, "admin": s.member.admin});
    let mut resp = super::json(StatusCode::OK, &body);
    resp = s.cookie.attach(resp);
    resp
}

fn redirect(location: &str, cookies: SetCookie) -> Response {
    let mut resp = (StatusCode::FOUND, [("Location", location.to_string())]).into_response();
    resp = cookies.attach(resp);
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    resp
}

fn bad_request() -> Response {
    api_error(StatusCode::BAD_REQUEST, "bad request")
}

/// The one mapping from `AuthError` to a response (03-ARCHITECTURE "Error mapping").
pub(crate) fn auth_error(e: AuthError) -> Response {
    match e {
        AuthError::SignInFailed(_) => {
            api_error(StatusCode::UNAUTHORIZED, "sign-in failed") // reason already logged
        }
        AuthError::SignInRequired => api_error(StatusCode::UNAUTHORIZED, "sign-in required"),
        AuthError::BadRequest => api_error(StatusCode::BAD_REQUEST, "bad request"),
        AuthError::Unavailable(cause) => {
            tracing::error!(target: "auth", "unavailable: {cause}");
            api_error(StatusCode::SERVICE_UNAVAILABLE, "service unavailable")
        }
    }
}
