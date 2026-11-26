//! `/api/me/mt5` pairing: the OTP mint, the code-authenticated pair, and the per-bridge revoke
//! (09-01; 09-ARCHITECTURE decisions 1 and 2).
//!
//! | Call | Request | Success | Failure |
//! |---|---|---|---|
//! | `POST /api/me/mt5/pairing-code` | — (session cookie) | 201 `{code, expires_at}` | 401; 503 db |
//! | `POST /api/me/mt5/pair` | `{code, label, url, token}` | 201 `{id, label, url, symbol_count, connected, revoke_code}` | 401 `code unknown or expired`; 422 `{error: <exact cause>}`; 503 db |
//! | `POST /api/me/mt5/paired/{id}/revoke` | `{revoke_code}` | 204 | 404 unknown id; 401 wrong secret; 503 db |
//!
//! The split is the phase's own decision: the MINT is session-gated (Settings holds the cookie),
//! while PAIR and REVOKE are secret-authenticated because their caller is the manager app on the
//! MT5 host, which holds no browser session (see `routes::gate::is_secret_authenticated`).
//! Pairing runs register's exact vet -> /health -> /symbols -> seal -> insert sequence through
//! [`super::mt5::register_for_user`], so D-032's guard, its order and its sealed storage hold on
//! both paths by construction rather than by a second copy free to drift.
//!
//! What a secret buys its holder is deliberately narrow: a pairing code registers ONE bridge for
//! the account that minted it and is spent doing so; a revoke code deletes exactly the bridge it
//! was issued for. Neither can list, read or test any bridge, and neither mints a session — no
//! cookie is produced anywhere in this module.

use super::auth::auth_error;
use super::mt5::{cause_response, db_down, register_for_user};
use crate::auth::ids::{RevokeHash, RevokeSecret};
use crate::auth::{AuthError, Viewer};
use crate::bridges::store;
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

/// The one refusal text for a bad pairing code. Unknown, expired and already-spent all land here
/// on purpose: telling a replayer "that code existed" is the timing oracle the single-use rule
/// denies, and a manager app's only sensible reaction to any of the three is "mint a fresh code
/// in Settings".
const PAIR_REFUSED: &str = "code unknown or expired";

/// `POST /api/me/mt5/pairing-code`: mint a ten-digit code for the signed-in session.
///
/// 201 `{code, expires_at}`. The code leaves the process exactly once, here; only its hash is
/// stored, and it dies with this session or at its TTL, whichever comes first.
pub async fn mint(State(st): State<AppState>, viewer: axum::Extension<Viewer>) -> Response {
    // Signed-out is 401 before anything else is read (the 05-02 AC-6 ordering).
    let member = match &viewer.0 {
        Viewer::Member(m) => m,
        Viewer::Anonymous => return auth_error(AuthError::SignInRequired),
    };
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match auth.mint_pairing_code(member).await {
        Ok((code, expires_at)) => super::json(
            StatusCode::CREATED,
            &json!({ "code": code, "expires_at": expires_at }),
        ),
        Err(e) => auth_error(e),
    }
}

#[derive(Deserialize)]
pub struct PairBody {
    code: String,
    label: String,
    url: String,
    token: String,
}

/// `POST /api/me/mt5/pair`: burn the code, resolve the minting user, then run register's
/// sequence unchanged (AC-3).
///
/// Order matters: the code is resolved and spent BEFORE the bridge is contacted, so one code
/// cannot be retried against a second URL after a refusal, and a bridge token cannot be probed
/// repeatedly with a code the caller still holds.
pub async fn pair(State(st): State<AppState>, axum::Json(body): axum::Json<PairBody>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let user = match auth.resolve_pairing_code(&body.code).await {
        Ok(u) => u,
        Err(AuthError::SignInFailed(_)) => {
            return super::api_error(StatusCode::UNAUTHORIZED, PAIR_REFUSED)
        }
        Err(e) => return auth_error(e),
    };
    let registered = match register_for_user(&st, user, &body.label, &body.url, &body.token).await {
        Ok(r) => r,
        Err(refusal) => return cause_response(refusal.status, &refusal.cause),
    };
    // The standing revoke secret (decision 2): one per bridge, shown here and never again.
    let secret = RevokeSecret::fresh();
    if let Err(e) = store::set_revoke_hash(auth.bridge_pool(), registered.id, &secret.hash()).await
    {
        // The bridge row now exists. Answering 201 with a secret that was not stored would hand
        // the app an unregister capability that silently does nothing, so this is a hard failure
        // and the row stays until the caller retries or deletes it from Settings.
        return db_down(&e);
    }
    let revoke_code = secret.encode();
    st.logs.add_secret(&revoke_code);
    st.logs.add_secret(&body.code);
    super::json(
        StatusCode::CREATED,
        &json!({
            "id": registered.id.to_string(),
            "label": registered.label,
            "url": registered.url,
            "symbol_count": registered.symbol_count,
            "connected": true,
            "revoke_code": revoke_code,
        }),
    )
}

#[derive(Deserialize)]
pub struct RevokeBody {
    revoke_code: String,
}

/// `POST /api/me/mt5/paired/{id}/revoke`: delete exactly the bridge this secret was issued for.
///
/// Three answers, in this order: 404 for an id no bridge carries (checked first, so an unknown id
/// costs no hash comparison and reveals nothing); 401 for a real bridge with the wrong secret;
/// 204 when the pair matches. The double-presentment case falls out of the delete itself: the
/// second call finds no bridge, so it is 404 and nothing is removed (G5).
pub async fn revoke(
    State(st): State<AppState>,
    Path(id): Path<Uuid>,
    axum::Json(body): axum::Json<RevokeBody>,
) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let pool = auth.bridge_pool();
    match store::exists(pool, id).await {
        Ok(true) => {}
        Ok(false) => return super::api_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => return db_down(&e),
    }
    // The secret itself is the authority: it identifies the bridge, so no session and no owner
    // check is needed to reach the delete (B9's doc records why that is safe).
    let presented = RevokeHash::of_secret(&body.revoke_code);
    match store::revoke_matches(pool, id, &presented).await {
        Ok(true) => {}
        Ok(false) => {
            // Burn the presented value out of the redaction set's blind spot and answer 401: a
            // wrong secret deletes nothing (AC-4, G5).
            st.logs.add_secret(&body.revoke_code);
            return super::api_error(
                StatusCode::UNAUTHORIZED,
                "revoke code unknown or already used",
            );
        }
        Err(e) => return db_down(&e),
    }
    match store::delete_by_id(pool, id).await {
        Ok(true) => {
            st.logs.add_secret(&body.revoke_code);
            StatusCode::NO_CONTENT.into_response()
        }
        // Another caller removed it between the two statements above: same answer as unknown.
        Ok(false) => super::api_error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => db_down(&e),
    }
}
