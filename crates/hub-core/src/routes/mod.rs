//! Route modules and the response helpers every route shares.
//!
//! API responses always carry `Content-Type: application/json` and `Cache-Control: no-store`
//! (`hub/server.py:1018-1024`); errors are `{"error": "<text>"}`.

pub mod auth;
pub mod gate;
pub mod health;
pub mod logs;
pub mod market;
pub mod me;
pub mod metrics;
pub mod orderflow;
pub mod table;
pub mod ws;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use hub_types::market::ErrorBody;
use serde::Serialize;

/// A JSON response with the API headers.
pub fn json(status: StatusCode, body: &impl Serialize) -> Response {
    match serde_json::to_vec(body) {
        Ok(bytes) => {
            let mut resp = (status, bytes).into_response();
            let h = resp.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            resp
        }
        Err(e) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("serialize: {e}"),
        ),
    }
}

/// `200` with a JSON body.
pub fn json_ok(body: &impl Serialize) -> Response {
    json(StatusCode::OK, body)
}

/// An error with the Python body shape.
pub fn api_error(status: StatusCode, msg: &str) -> Response {
    let body = ErrorBody {
        error: msg.to_string(),
    };
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{\"error\":\"error\"}".to_vec());
    let mut resp = (status, bytes).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}
