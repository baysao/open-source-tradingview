//! `/api/me/mt5*` — the bridge registration surface (07-03; D-032's route table, 07-01's
//! store, guard and client underneath). Every route scopes by the session's own user id: a
//! foreign id is 404, never 403 (G5; the admin-gate convention), and the token never leaves
//! the server.
//!
//! | Call | Request | Success | Failure |
//! |---|---|---|---|
//! | `POST /api/me/mt5` | `{label, url, token}` | 201 `{id, label, url, symbol_count, connected}` | 422 `{error: <exact cause>}`; 401; 503 db |
//! | `GET /api/me/mt5` | — | 200 `{bridges: [{id, label, url, connected, last_ok_at, last_error, symbol_count}]}` | 401; 503 db |
//! | `POST /api/me/mt5/{id}/test` | — | 200 `{connected, symbol_count}` | 502 `{error: <exact cause>}`; 404; 401 |
//! | `DELETE /api/me/mt5/{id}` | — | 204 | 404; 401 |
//!
//! Registration vets the URL (syntax + guard, `auth_enabled = true`: loopback refused here,
//! AC-2's auth-on half) and then live-tests `/health` followed by `/symbols` (CUJ 7's order);
//! success seals the token, inserts the row and caches the symbols answer (D-032b: the cache
//! is written at every successful Test, never read live by `/api/symbols`).

use super::auth::auth_error;
use crate::auth::{AuthError, Viewer};
use crate::bridges::crypto;
use crate::bridges::guard::vet_url;
use crate::bridges::store::{self, NewBridge};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// The session's user id, or the 401 a signed-out caller gets before any other input is read
/// (the 05-02 AC-6 ordering, applied to the bridge surface). The response is boxed: a large
/// `Response` in the `Err` slot trips clippy's `result_large_err`, and only the failure path
/// pays for the box.
fn member(viewer: &Viewer) -> Result<Uuid, Box<Response>> {
    match viewer {
        Viewer::Member(m) => Ok(m.user.id),
        Viewer::Anonymous => Err(Box::new(auth_error(AuthError::SignInRequired))),
    }
}

/// The bridge-token key derived from this state's `VTV_SECRET_KEY` (07-01's key separation:
/// `HMAC-SHA256(secret, "mt5-bridge-token")`, never the cookie-MAC key). Shared by the bridge
/// routes and the ws bridge path (`routes::market`, `routes::ws`).
pub(crate) fn bridge_key(st: &AppState) -> [u8; 32] {
    match &st.cfg.auth {
        crate::config::AuthMode::Google(s) => crypto::bridge_key(s.secret_key.bytes()),
        crate::config::AuthMode::Off => unreachable!("mounted only when hosted"),
    }
}

/// A failed registration or Test: the exact cause text verbatim (CUJ 7, D-032e).
pub(crate) fn cause_response(status: StatusCode, cause: &str) -> Response {
    super::api_error(status, cause)
}

/// A bridge-store error is `service unavailable` (the cause logs under `auth`, never renders;
/// the same mapping the saved-state routes use for their storage).
pub(crate) fn db_down(e: &sqlx::Error) -> Response {
    auth_error(AuthError::Unavailable(format!("mt5 store: {e}")))
}

/// How many symbols a cached `/symbols` answer holds. The bridge returns a bare array; a
/// cache in any other shape counts as zero rather than guessing.
fn symbol_count_of(symbols_json: Option<&str>) -> usize {
    symbols_json
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_array().map(|a| a.len()))
        .unwrap_or(0)
}

#[derive(Deserialize)]
pub struct RegisterBody {
    label: String,
    url: String,
    token: String,
}

/// What a successful registration produced, before it is rendered: the row's identity and the
/// live test's answer. Both the session route (`register`) and the code-authenticated pairing
/// route (`routes::pairing::pair`) end here, so vet -> /health -> /symbols -> seal -> insert is
/// ONE sequence with one set of refusal texts (AC-3; a copy would drift).
pub(crate) struct Registered {
    pub(crate) id: Uuid,
    pub(crate) label: String,
    pub(crate) url: String,
    pub(crate) symbol_count: usize,
}

/// The refusal a registration answers with: the exact cause text and the status carrying it.
pub(crate) struct RegisterRefusal {
    pub(crate) status: StatusCode,
    pub(crate) cause: String,
}

/// Vet, live-test in CUJ 7's order, seal, insert: the one registration sequence (07-03's
/// `register` body, lifted verbatim so pairing cannot fork it).
///
/// The guard runs first (a refused URL never reaches the network, AC-2's auth-on half), the
/// token goes only into the Authorization header, and no error path composes it into text
/// (07-01's tracked note). A store failure is [`RegisterRefused`] with 503, so the caller keeps
/// its own error mapping.
pub(crate) async fn register_for_user(
    st: &AppState,
    user: Uuid,
    label: &str,
    url: &str,
    token: &str,
) -> Result<Registered, RegisterRefusal> {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    // The id is minted before the calls so the client's per-bridge semaphore has its key; the
    // same id lands in the row on success.
    let id = Uuid::now_v7();
    // Guard first: a refused URL never reaches the network (AC-2, auth-on half).
    let vetted = match vet_url(url, true).await {
        Ok(v) => v,
        Err(e) => {
            return Err(RegisterRefusal {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                cause: e.to_string(),
            })
        }
    };
    // Live test in CUJ 7's order: /health then /symbols.
    if let Err(e) = st.bridges.client.health(&id, &vetted, token).await {
        return Err(RegisterRefusal {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            cause: e.to_string(),
        });
    }
    let symbols = match st.bridges.client.symbols(&id, &vetted, token).await {
        Ok(v) => v,
        Err(e) => {
            return Err(RegisterRefusal {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                cause: e.to_string(),
            })
        }
    };
    let symbol_count = symbols.as_array().map(|a| a.len()).unwrap_or(0);
    let symbols_json = serde_json::to_string(&symbols).ok();
    let token_enc = crypto::seal(&bridge_key(st), token);
    let now = chrono::Utc::now();
    let row = NewBridge {
        id,
        user_id: user,
        label: label.trim().to_string(),
        url: vetted.url.to_string(),
        token_enc,
        symbols_json: symbols_json.clone(),
        last_ok_at: Some(now),
        last_error: None,
        created_at: now,
    };
    if let Err(e) = store::insert(auth.bridge_pool(), &row).await {
        return Err(RegisterRefusal {
            status: StatusCode::SERVICE_UNAVAILABLE,
            cause: format!("mt5 store: {e}"),
        });
    }
    // The token and the URL's query string join the ring's redaction set (AC-5): a hostile
    // bridge echoing the Authorization value into its error text cannot land it in a log.
    st.logs.add_secret(token);
    if let Some(q) = vetted.url.query().map(|s| s.to_string()) {
        st.logs.add_secret(&q);
    }
    Ok(Registered {
        id,
        label: row.label,
        url: row.url,
        symbol_count,
    })
}

/// `POST /api/me/mt5`: vet, live-test, seal, insert. 201 with the listing fields or 422 with
/// the exact cause (a guard refusal, a DNS miss, a wrong token, a bridge error — whatever
/// CUJ 7's Test screen must show verbatim).
pub async fn register(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    axum::Json(body): axum::Json<RegisterBody>,
) -> Response {
    let user = match member(&viewer.0) {
        Ok(u) => u,
        Err(resp) => return *resp,
    };
    match register_for_user(&st, user, &body.label, &body.url, &body.token).await {
        Ok(r) => super::json(
            StatusCode::CREATED,
            &json!({
                "id": r.id.to_string(),
                "label": r.label,
                "url": r.url,
                "symbol_count": r.symbol_count,
                "connected": true,
            }),
        ),
        Err(refusal) if refusal.status == StatusCode::SERVICE_UNAVAILABLE => {
            auth_error(AuthError::Unavailable(refusal.cause))
        }
        Err(refusal) => cause_response(refusal.status, &refusal.cause),
    }
}

/// `GET /api/me/mt5`: this session's bridges, oldest first, the token absent by construction
/// (`BridgeListing` carries no token field).
pub async fn list(State(st): State<AppState>, viewer: axum::Extension<Viewer>) -> Response {
    let user = match member(&viewer.0) {
        Ok(u) => u,
        Err(resp) => return *resp,
    };
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match store::list_for(auth.bridge_pool(), user).await {
        Ok(rows) => {
            let bridges: Vec<_> = rows
                .into_iter()
                .map(|b| {
                    json!({
                        "id": b.id.to_string(),
                        "label": b.label,
                        "url": b.url,
                        "connected": b.last_ok_at.is_some(),
                        "last_ok_at": b.last_ok_at,
                        "last_error": b.last_error,
                        "symbol_count": symbol_count_of(b.symbols_json.as_deref()),
                    })
                })
                .collect();
            super::json_ok(&json!({ "bridges": bridges }))
        }
        Err(e) => db_down(&e),
    }
}

/// `POST /api/me/mt5/{id}/test`: re-test a bridge this session owns (a foreign id is 404
/// before anything is dialed, G5), refresh the cached symbols, and record the result. The
/// route table's two answers: 200 with the result, 502 `{error: <exact cause>}`. A failed
/// test keeps the last successful cache and `last_ok_at` (D-032b: the cache is from the last
/// successful test, so one bad re-test does not blank the picker); only `last_error` moves.
pub async fn test(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    Path(id): Path<Uuid>,
) -> Response {
    let user = match member(&viewer.0) {
        Ok(u) => u,
        Err(resp) => return *resp,
    };
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let pool = auth.bridge_pool();
    let Some(row) = (match store::get_owned(pool, user, id).await {
        Ok(r) => r,
        Err(e) => return db_down(&e),
    }) else {
        return super::api_error(StatusCode::NOT_FOUND, "not found");
    };
    let token = match crypto::open(&bridge_key(&st), &row.token_enc) {
        Ok(t) => t,
        Err(_) => {
            return super::api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the bridge token seal is unreadable",
            )
        }
    };
    // A stored URL re-vets with auth on: a refusal lands in `last_error` and answers 502 with
    // its exact cause, like any other test failure.
    let vetted = match vet_url(&row.url, true).await {
        Ok(v) => v,
        Err(e) => {
            let cause = e.to_string();
            if let Err(db) = store::set_test_result(
                pool,
                user,
                id,
                row.symbols_json.as_deref(),
                row.last_ok_at,
                Some(&cause),
            )
            .await
            {
                return db_down(&db);
            }
            return cause_response(StatusCode::BAD_GATEWAY, &cause);
        }
    };
    let outcome = async {
        st.bridges.client.health(&id, &vetted, &token).await?;
        st.bridges.client.symbols(&id, &vetted, &token).await
    }
    .await;
    match outcome {
        Ok(symbols) => {
            let symbol_count = symbols.as_array().map(|a| a.len()).unwrap_or(0);
            let symbols_json = serde_json::to_string(&symbols).ok();
            if let Err(e) = store::set_test_result(
                pool,
                user,
                id,
                symbols_json.as_deref(),
                Some(chrono::Utc::now()),
                None,
            )
            .await
            {
                return db_down(&e);
            }
            // The re-tested token and URL query re-join the redaction set (a no-op when
            // unchanged; AC-5 holds whatever the row's history).
            st.logs.add_secret(&token);
            if let Some(q) = vetted.url.query().map(|s| s.to_string()) {
                st.logs.add_secret(&q);
            }
            super::json_ok(&json!({ "connected": true, "symbol_count": symbol_count }))
        }
        Err(e) => {
            let cause = e.to_string();
            if let Err(db) = store::set_test_result(
                pool,
                user,
                id,
                row.symbols_json.as_deref(),
                row.last_ok_at,
                Some(&cause),
            )
            .await
            {
                return db_down(&db);
            }
            cause_response(StatusCode::BAD_GATEWAY, &cause)
        }
    }
}

/// `DELETE /api/me/mt5/{id}`: remove a bridge this session owns; another user's id (and an
/// unknown one) is 404, and nothing of theirs is ever deleted (G5).
pub async fn delete(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    Path(id): Path<Uuid>,
) -> Response {
    let user = match member(&viewer.0) {
        Ok(u) => u,
        Err(resp) => return *resp,
    };
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match store::delete_owned(auth.bridge_pool(), user, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => super::api_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => db_down(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridges::client::BridgeError;

    #[test]
    fn symbol_count_reads_the_cached_array() {
        // The bridge's /symbols answer is a bare array (bridge/mt5_bridge.py:250).
        assert_eq!(symbol_count_of(Some(r#"[{"name":"XAUUSD"}]"#)), 1);
        assert_eq!(symbol_count_of(Some("[]")), 0);
        // Any other shape counts as zero, never guessed.
        assert_eq!(symbol_count_of(Some("{\"symbols\":[]}")), 0);
        assert_eq!(symbol_count_of(Some("not json")), 0);
        assert_eq!(symbol_count_of(None), 0);
    }

    #[test]
    fn exact_cause_texts_are_the_display_strings() {
        // The 422/502 body carries Display verbatim (CUJ 7); the strings are the UI's table.
        assert_eq!(
            BridgeError::Auth.to_string(),
            "the bridge refused the token"
        );
        assert_eq!(
            BridgeError::Dns.to_string(),
            "the bridge host did not resolve"
        );
        assert_eq!(
            BridgeError::Timeout.to_string(),
            "the bridge did not answer in time"
        );
    }
}
