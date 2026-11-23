//! `GET /api/health`.

use super::json_ok;
use crate::AppState;
use axum::extract::State;
use axum::response::Response;

pub async fn health(State(st): State<AppState>) -> Response {
    json_ok(&st.feeds.health())
}
