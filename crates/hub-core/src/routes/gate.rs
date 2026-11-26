//! The hosted-mode gate: the only place the D-010 and admin rules live. One middleware keyed by
//! [`area`]; mounted on the whole router when hosted (03-ARCHITECTURE decision 2).
//!
//! Order: a path outside the three areas passes with no database hit; `/admin/*` resolves the
//! viewer and answers 404 unless admin (before the Origin rule, so an outsider learns nothing);
//! any non-GET in an area needs `Origin == PUBLIC_ORIGIN` (missing refused, 403) except the two
//! secret-authenticated pairing paths; a non-GET that declares a body needs JSON (415) under
//! 300,000 bytes (413); then the viewer is resolved once and inserted as `Extension<Viewer>`;
//! after the handler, the slide's cookie is appended unless the response already sets the session
//! cookie (sign-in, logout, delete win).

use crate::auth::{AuthError, SetCookie, Viewer};
use crate::routes::api_error;
use crate::AppState;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;

/// The session cookie name (`auth` owns the format; the gate only reads the value).
const SESSION_COOKIE: &str = "__Host-vtv_session";

/// The three gated prefixes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Area {
    Auth,
    Me,
    Admin,
}

/// `"/auth/"` prefix, `"/admin/"` prefix, `"/api/me"` exactly or `"/api/me/"` prefix (never
/// `"/api/metrics"`).
pub(crate) fn area(path: &str) -> Option<Area> {
    if path.starts_with("/auth/") {
        return Some(Area::Auth);
    }
    if path.starts_with("/admin/") {
        return Some(Area::Admin);
    }
    if path == "/api/me" || path.starts_with("/api/me/") {
        return Some(Area::Me);
    }
    None
}

/// The session cookie value of the request, or `None`.
fn session_cookie(req: &Request<Body>) -> Option<String> {
    for v in req.headers().get_all(header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for pair in s.split(';') {
            let pair = pair.trim();
            if let Some(rest) = pair.strip_prefix(SESSION_COOKIE) {
                if let Some(value) = rest.strip_prefix('=') {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Whether the request declares a body (content-length over 0, or chunked). DELETE carries none.
fn declares_body(req: &Request<Body>) -> bool {
    let declared = req
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    declared > 0 || req.headers().contains_key(header::TRANSFER_ENCODING)
}

/// The two pairing routes the manager app calls, which authenticate by a presented SECRET
/// rather than by the session cookie: `POST /api/me/mt5/pair` (a one-time pairing code) and
/// `POST /api/me/mt5/paired/{id}/revoke` (a per-bridge revoke secret). 09-01.
///
/// The Origin rule stops a hostile page from riding the user's cookie (D-010). These paths never
/// read that cookie: their credential is the body secret, so a cross-site caller has nothing to
/// ride, while the rule as written would refuse the one legitimate caller — the Rust manager app
/// on the MT5 host, which sends no `Origin` header at all.
///
/// Matched structurally, never by prefix: `/api/me/mt5/pairing-code` (the mint, which IS
/// session-gated) and every other `/api/me/mt5*` path keep the rule. A prefix test would quietly
/// unguard the mint and each future sibling.
fn is_secret_authenticated(path: &str) -> bool {
    if path == "/api/me/mt5/pair" {
        return true;
    }
    // `/api/me/mt5/paired/{id}/revoke`: five leading segments, an opaque non-empty id, a fixed
    // tail. `split('/')` on a rooted path always yields a leading empty segment.
    let p: Vec<&str> = path.split('/').collect();
    matches!(p.as_slice(), ["", "api", "me", "mt5", "paired", id, "revoke"] if !id.is_empty())
}

/// The gate. Mounted on the whole router only when hosted (local mode has no gated route).
pub async fn gate(State(st): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let Some(area) = area(req.uri().path()) else {
        return next.run(req).await;
    };
    let Some(auth) = st.auth.as_ref() else {
        // Unreachable through `app()` (the gate mounts only when hosted), but never a panic.
        return api_error(StatusCode::NOT_FOUND, "not found");
    };

    // `/admin/*`: admin or 404, decided before the Origin rule (an outsider learns nothing).
    // Every other area: the viewer is resolved below, after the Origin and body rules.
    let (mut viewer, mut slide): (Viewer, Option<SetCookie>) = (Viewer::Anonymous, None);
    if area == Area::Admin {
        match auth.resolve(session_cookie(&req).as_deref()).await {
            Ok((v, s)) => {
                let admin = matches!(&v, Viewer::Member(m) if m.admin);
                if !admin {
                    return api_error(StatusCode::NOT_FOUND, "not found");
                }
                (viewer, slide) = (v, s);
            }
            Err(AuthError::Unavailable(cause)) => {
                tracing::error!(target: "auth", "unavailable: {cause}");
                return api_error(StatusCode::SERVICE_UNAVAILABLE, "service unavailable");
            }
            Err(_) => return api_error(StatusCode::NOT_FOUND, "not found"),
        }
    }

    if req.method() != axum::http::Method::GET {
        // D-010, hosted strictness: a missing Origin is refused too (curl and agents included).
        // The secret-authenticated pairing paths are exempt (see `is_secret_authenticated`): they
        // carry their own credential in the body and never read the session cookie.
        let origin_ok = is_secret_authenticated(req.uri().path())
            || req
                .headers()
                .get(header::ORIGIN)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|o| o == st.cfg.public_origin);
        if !origin_ok {
            tracing::debug!(target: "auth", "origin refused");
            return api_error(StatusCode::FORBIDDEN, "forbidden origin");
        }
        // A request that declares a body must be JSON under 300,000 bytes.
        if declares_body(&req) {
            let ctype = req
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if !ctype.contains("application/json") {
                return api_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "JSON only");
            }
            let declared = req
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            if declared > 300_000 {
                return api_error(StatusCode::PAYLOAD_TOO_LARGE, "too large");
            }
        }
    }

    // The viewer, resolved once, for the non-admin areas (the admin area has it already).
    if area != Area::Admin {
        match auth.resolve(session_cookie(&req).as_deref()).await {
            Ok((v, s)) => {
                viewer = v;
                slide = s;
            }
            Err(AuthError::Unavailable(cause)) => {
                // A database outage answers 503; signed-out is never faked.
                tracing::error!(target: "auth", "unavailable: {cause}");
                return api_error(StatusCode::SERVICE_UNAVAILABLE, "service unavailable");
            }
            // A bad cookie is simply signed out; handlers needing a member re-check.
            Err(_) => {}
        }
    }

    let mut req = req;
    req.extensions_mut().insert(viewer);
    let mut resp = next.run(req).await;
    if let Some(c) = slide {
        // The slide cookie never overwrites a cookie the handler set (sign-in, logout, delete win).
        if !SetCookie::sets_session(&resp) {
            resp = c.attach(resp);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The area decision is the gate's key (architecture decision 2): the prefix rules match
    /// D-010's own wording, and `/api/metrics` must never match `/api/me`.
    #[test]
    fn the_area_prefixes_match_the_spec() {
        assert_eq!(area("/auth/me"), Some(Area::Auth));
        assert_eq!(area("/auth/google/start"), Some(Area::Auth));
        assert_eq!(area("/admin/logs"), Some(Area::Admin));
        assert_eq!(area("/admin/logs/journal"), Some(Area::Admin));
        assert_eq!(area("/api/me"), Some(Area::Me));
        assert_eq!(area("/api/me/extra"), Some(Area::Me));
        assert_eq!(area("/api/metrics"), None, "not a prefix of /api/me");
        assert_eq!(area("/api/logs"), None);
        assert_eq!(area("/hub.html"), None);
        assert_eq!(area("/ws"), None);
        assert_eq!(area("/authx"), None, "the slash ends the prefix");
        assert_eq!(area("/adminx"), None);
    }

    /// The Origin exemption is exactly two paths, and it must never widen (09-01 AC-5).
    #[test]
    fn only_the_two_secret_authenticated_paths_skip_the_origin_rule() {
        assert!(is_secret_authenticated("/api/me/mt5/pair"));
        assert!(is_secret_authenticated(
            "/api/me/mt5/paired/0192f3a4-0000-7000-8000-000000000000/revoke"
        ));
        // The MINT is session-gated and must keep the rule: a prefix test here would lose it.
        assert!(!is_secret_authenticated("/api/me/mt5/pairing-code"));
        // No sibling widens either: the register/list/test/delete surface is untouched.
        assert!(!is_secret_authenticated("/api/me/mt5"));
        assert!(!is_secret_authenticated("/api/me/mt5/paired"));
        assert!(!is_secret_authenticated("/api/me/mt5/paired/x"));
        assert!(!is_secret_authenticated("/api/me/mt5/paired//revoke"));
        assert!(!is_secret_authenticated(
            "/api/me/mt5/paired/x/revoke/extra"
        ));
        assert!(!is_secret_authenticated("/api/me/state"));
        assert!(!is_secret_authenticated("/api/me/mt5/pair/extra"));
    }
}
