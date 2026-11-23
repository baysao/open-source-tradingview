//! `GET /api/logs?since=&brief` and `POST /api/logs/journal` (`hub/server.py:911-914`, `1002-1004`).

use super::json_ok;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use hub_types::logs::LogsResponse;
use std::collections::HashMap;

/// `since` counts only when it is all digits (Python `isdigit`); `brief` by presence.
pub async fn logs(
    State(st): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let since = q
        .get("since")
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let brief = q.contains_key("brief");
    json_ok(&LogsResponse {
        snapshot: st.logs.snapshot(since, brief),
        journal_raw: false,
    })
}

/// The panel's "chart reads" toggle. A no-op stub, as in Python, behind the POST guard.
pub async fn journal(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(resp) = crate::net::post_guard(&st.cfg, &headers, body.len()) {
        return *resp;
    }
    json_ok(&serde_json::json!({"ok": true}))
}
