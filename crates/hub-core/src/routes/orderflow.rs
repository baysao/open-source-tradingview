//! `GET /api/orderflow/{delta,profile,footprint}` (`hub/server.py:921-968`): one handler mounted on
//! the three paths. Any failure answers 502 `{"error": "<Class>: <msg>"}` and logs one warn line
//! (`orderflow <kind> <symbol> <tf>: <msg>`, at most every 30 s per text), as Python.

use super::{api_error, json_ok};
use crate::orderflow::{Ask, Kind};
use crate::AppState;
use axum::http::StatusCode;
use axum::response::Response;
use std::collections::HashMap;

pub async fn serve(kind: Kind, st: AppState, q: HashMap<String, String>) -> Response {
    let of = &st.orderflow;
    let res = match Ask::parse(kind, &q, of.now_s()) {
        Ok(ask) => of.answer(&ask).await,
        Err(e) => Err(e),
    };
    match res {
        Ok(reply) => json_ok(&reply),
        Err(e) => {
            let arg = |k: &str| {
                q.get(k)
                    .filter(|v| !v.is_empty())
                    .map_or("None", String::as_str)
                    .to_string()
            };
            tracing::warn!(target: "server", every = 30u64, "orderflow {kind} {} {}: {}", arg("symbol"), arg("tf"), e.msg());
            api_error(StatusCode::BAD_GATEWAY, &e.to_string())
        }
    }
}
