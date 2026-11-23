//! The static file rule, ported from `static_allowed` (`hub/server.py:886-898`).
//!
//! Only top-level files and files directly under `vendor/` with a web extension are served, so a
//! secret placed in the static folder (`.env`, logs, caches, Python sources) answers 404. The same
//! rule serves GET and HEAD (the Python stdlib let `HEAD` bypass the check).

use axum::body::Body;
use axum::http::{header, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use percent_encoding::percent_decode_str;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use tower_http::services::ServeFile;

const STATIC_EXT: [&str; 8] = [
    ".html", ".js", ".css", ".png", ".svg", ".ico", ".woff", ".woff2",
];

/// The file a request path may serve, relative to the static root, or `None` (404).
///
/// Percent-decodes first, drops the query, maps `/` to `index.html`, refuses any segment that
/// starts with `.` and any backslash, and allows one segment, or `vendor/<file>`, with a web
/// extension. Pure.
pub fn resolve(raw_path: &str) -> Option<PathBuf> {
    let path_only = raw_path.split(['?', '#']).next().unwrap_or("");
    let decoded = percent_decode_str(path_only).decode_utf8().ok()?;
    if decoded.is_empty() || decoded == "/" {
        return Some(PathBuf::from("index.html"));
    }
    if decoded.contains('\\') {
        return None;
    }
    let parts: Vec<&str> = decoded.split('/').filter(|p| !p.is_empty()).collect();
    if parts.iter().any(|p| p.starts_with('.')) {
        return None;
    }
    let allowed_shape = parts.len() == 1 || (parts.len() == 2 && parts[0] == "vendor");
    if !allowed_shape {
        return None;
    }
    let last = parts.last()?.to_ascii_lowercase();
    if !STATIC_EXT.iter().any(|ext| last.ends_with(ext)) {
        return None;
    }
    Some(parts.iter().collect())
}

/// Serves `resolve(path)` from `root` for GET and HEAD; anything else is the JSON 404.
pub async fn serve(root: &Path, req: Request<Body>) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    }
    let Some(rel) = resolve(req.uri().path()) else {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    };
    let file = root.join(rel);
    if !file.is_file() {
        return crate::routes::api_error(StatusCode::NOT_FOUND, "not found");
    }
    match ServeFile::new(file).oneshot(req).await {
        Ok(resp) => {
            let mut resp = resp.into_response();
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-cache, must-revalidate"),
            );
            resp
        }
        Err(_) => crate::routes::api_error(StatusCode::NOT_FOUND, "not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use std::path::PathBuf;

    #[test]
    fn table() {
        let allowed = [
            ("/", "index.html"),
            ("", "index.html"),
            ("/hub.html", "hub.html"),
            ("/hub.html?x=1", "hub.html"),
            ("/vendor/klinecharts.min.js", "vendor/klinecharts.min.js"),
            ("/VELA.BUNDLE.JS", "VELA.BUNDLE.JS"),
        ];
        for (raw, want) in allowed {
            assert_eq!(resolve(raw), Some(PathBuf::from(want)), "{raw}");
        }
        let refused = [
            "/.env",
            "/%2e%2e/LICENSE",
            "/%2e%2e%2fLICENSE",
            "/a%5cb.js",
            "/a\\b.js",
            "/vendor/a/b.js",
            "/server.py",
            "/x.txt",
            "/logs/hub.log",
            "/.git/config",
            "/vendor/.hidden.js",
            "/tests/pine_convert_harness.js",
        ];
        for raw in refused {
            assert_eq!(resolve(raw), None, "{raw}");
        }
    }
}
