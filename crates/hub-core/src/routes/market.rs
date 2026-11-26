//! `/api/status`, `/api/symbols`, `/api/instruments`, `/api/bars`.

use super::auth::auth_error;
use super::mt5::bridge_key;
use super::{api_error, json_ok};
use crate::auth::{AuthError, Viewer};
use crate::bars_cache::Freshness;
use crate::bridges::client::BarsQuery as BridgeBarsQuery;
use crate::bridges::crypto;
use crate::bridges::guard::vet_url;
use crate::bridges::store;
use crate::market::BarsQuery;
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use hub_types::market::Bar;
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn status(State(st): State<AppState>) -> Response {
    json_ok(&st.catalog.status())
}

/// `GET /api/symbols`: today's anonymous body when there is nothing to merge (G1, byte for
/// byte), and for a signed-in owner each tested bridge's cached symbols appended as D-032a's
/// entries (`source: "mt5"`, the bridge id per entry, D-032b: the cache, never a live call).
pub async fn symbols(
    State(st): State<AppState>,
    viewer: Option<axum::Extension<Viewer>>,
) -> Response {
    let owner = viewer.as_ref().and_then(|e| match &e.0 {
        Viewer::Member(m) => Some(m.user.id),
        Viewer::Anonymous => None,
    });
    let extra = match owner {
        Some(user) if !st.cfg.auth.is_local() => bridge_symbol_entries(&st, user).await,
        _ => Vec::new(),
    };
    if extra.is_empty() {
        // The anonymous path is today's code path, untouched (AC-4's anonymous half).
        return json_ok(&st.catalog.symbols());
    }
    json_ok(&st.catalog.symbols_with(extra))
}

/// The session owner's cached bridge symbols, as D-032a's entries: the bridge's `/symbols`
/// row verbatim plus `source: "mt5"` and the bridge id. A bridge with no cached answer
/// (never successfully tested) contributes nothing; a storage error degrades to the
/// anonymous shape with a warning rather than breaking the picker.
async fn bridge_symbol_entries(st: &AppState, user: Uuid) -> Vec<serde_json::Value> {
    let auth = match st.auth.as_ref() {
        Some(a) => a,
        None => return Vec::new(),
    };
    let rows = match store::list_for(auth.bridge_pool(), user).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(target: "server", "symbols merge: {e}");
            return Vec::new();
        }
    };
    rows.iter()
        .flat_map(|b| {
            b.symbols_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .and_then(|v| v.as_array().cloned())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|row| {
                            Some(json!({
                                "name": row.get("name")?,
                                "desc": row.get("desc")?,
                                "path": row.get("path")?,
                                "digits": row.get("digits")?,
                                "source": "mt5",
                                "bridge": b.id.to_string(),
                            }))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

pub async fn instruments(State(st): State<AppState>) -> Response {
    json_ok(&*st.catalog.instruments())
}

/// A bare JSON array of bars, oldest first; any failure is 502 `{"error": ...}` (Python
/// parity). `source=mt5` with a `bridge` id (or `local` when auth is off, D-033) reads the
/// bridge's own `/bars` through the pinned client; every other query is today's cache path,
/// untouched (D-009 parity).
pub async fn bars(
    State(st): State<AppState>,
    viewer: Option<axum::Extension<Viewer>>,
    Query(p): Query<HashMap<String, String>>,
) -> Response {
    match p.get("bridge").map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(raw) => bars_bridge(&st, viewer, raw, &p).await,
        None => {
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
    }
}

/// The bridge path (AC-3's REST half): signed-out is 401 before the id is parsed (the 05-02
/// AC-6 ordering), a foreign or unknown id is 404 before anything is dialed (G5), the stored
/// URL re-vets with the registration's rules, the rows map field-for-field onto `Bar`, and
/// every failure carries the exact cause verbatim (D-032e).
async fn bars_bridge(
    st: &AppState,
    viewer: Option<axum::Extension<Viewer>>,
    raw: &str,
    p: &HashMap<String, String>,
) -> Response {
    // tf and limit keep their existing bounds and error texts; the symbol is the bridge's own
    // (never a Binance ticker validation — bridge symbols are the terminal's).
    let tf_raw = p
        .get("tf")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("M1");
    let Some(tf) = hub_types::market::Tf::parse(tf_raw) else {
        return api_error(StatusCode::BAD_GATEWAY, &format!("'{tf_raw}'"));
    };
    let symbol = p
        .get("symbol")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("XAUUSD");
    let limit = p
        .get("limit")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1000)
        .clamp(1, 5000);
    let to = p.get("to").and_then(|s| s.parse::<i64>().ok());

    let (bridge_id, vetted, token) = if raw == "local" {
        // The env-configured local bridge (D-033): auth off only. The vet re-runs at call
        // time, so the loopback exception (and only it) bites here.
        let local = match (&st.bridges.local, st.cfg.auth.is_local()) {
            (Some(l), true) => l,
            _ => return api_error(StatusCode::NOT_FOUND, "not found"),
        };
        let vetted = match vet_url(&local.url, false).await {
            Ok(v) => v,
            Err(e) => return api_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        (
            crate::bridges::LOCAL_SEMAPHORE_ID,
            vetted,
            local.token.expose().to_string(),
        )
    } else {
        let user = match viewer.as_ref().and_then(|e| match &e.0 {
            Viewer::Member(m) => Some(m.user.id),
            Viewer::Anonymous => None,
        }) {
            Some(u) => u,
            None => return api_error(StatusCode::UNAUTHORIZED, "sign-in required"),
        };
        let Ok(id) = Uuid::parse_str(raw) else {
            return api_error(StatusCode::NOT_FOUND, "not found");
        };
        let auth = st.auth.as_ref().expect("mounted only when hosted");
        let row = match store::get_owned(auth.bridge_pool(), user, id).await {
            Ok(Some(r)) => r,
            Ok(None) => return api_error(StatusCode::NOT_FOUND, "not found"),
            Err(e) => return auth_error(AuthError::Unavailable(format!("mt5 store: {e}"))),
        };
        let token = match crypto::open(&bridge_key(st), &row.token_enc) {
            Ok(t) => t,
            Err(_) => {
                return api_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the bridge token seal is unreadable",
                )
            }
        };
        let vetted = match vet_url(&row.url, true).await {
            Ok(v) => v,
            Err(e) => return api_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        };
        (id, vetted, token)
    };
    let bq = BridgeBarsQuery {
        symbol,
        tf: tf.as_str(),
        limit,
        to,
    };
    match st
        .bridges
        .client
        .bars(&bridge_id, &vetted, &token, bq)
        .await
    {
        Ok(v) => match serde_json::from_value::<Vec<Bar>>(v) {
            Ok(bars) => json_ok(&bars),
            Err(e) => api_error(
                StatusCode::BAD_GATEWAY,
                &format!("the bridge answered a shape /bars does not send: {e}"),
            ),
        },
        Err(e) => api_error(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_rows_map_field_for_field() {
        // The bridge's /bars row (bridge/mt5_bridge.py:141 to_bar) onto Bar, every field named.
        let row: serde_json::Value = serde_json::json!({
            "time": 1_700_000_000, "open": 2071.5, "high": 2072.0, "low": 2070.9,
            "close": 2071.8, "volume": 321.0
        });
        let bar: Bar = serde_json::from_value(row).unwrap();
        assert_eq!(bar.time, 1_700_000_000, "time");
        assert_eq!(bar.open, 2071.5, "open");
        assert_eq!(bar.high, 2072.0, "high");
        assert_eq!(bar.low, 2070.9, "low");
        assert_eq!(bar.close, 2071.8, "close");
        assert_eq!(bar.volume, 321.0, "volume");
    }
}
