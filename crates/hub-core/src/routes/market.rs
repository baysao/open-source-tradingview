//! `/api/status`, `/api/symbols`, `/api/instruments`, `/api/bars`.

use super::{api_error, json_ok};
use crate::bars_cache::Freshness;
use crate::market::BarsQuery;
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use std::collections::HashMap;

pub async fn status(State(st): State<AppState>) -> Response {
    json_ok(&st.catalog.status())
}

pub async fn symbols(State(st): State<AppState>) -> Response {
    json_ok(&st.catalog.symbols())
}

pub async fn instruments(State(st): State<AppState>) -> Response {
    json_ok(&*st.catalog.instruments())
}

/// A bare JSON array of bars, oldest first; any failure is 502 `{"error": ...}` (Python parity).
pub async fn bars(
    State(st): State<AppState>,
    Query(p): Query<HashMap<String, String>>,
) -> Response {
    let q = match BarsQuery::from_params(&p) {
        Ok(q) => q,
        Err(e) => return api_error(StatusCode::BAD_GATEWAY, &e.0),
    };
    match st.bars.get(&q, Freshness::Cached).await {
        Ok(bars) => json_ok(&*bars),
        Err(e) => {
            tracing::warn!(target: "server", every = 30, "{} {} {}: bars request failed: {}", q.source, q.symbol, q.tf, e);
            api_error(StatusCode::BAD_GATEWAY, &e.0)
        }
    }
}
