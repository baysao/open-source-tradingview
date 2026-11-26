//! `GET /auth/me` and `DELETE /api/me` (D-017: `/auth/me` carries `client_id` when hosted).
//!
//! The handler is mounted in both modes. In local mode it answers `{"auth": false}`; in hosted
//! mode the gate has resolved the viewer into the extension, so the handler renders from it.

use super::auth::auth_error;
use crate::auth::Viewer;
use crate::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// `GET /auth/me`: local mode `{"auth": false}`; hosted `{"auth": true, "user": null|"...",
/// "admin": bool, "client_id": "..."}` (D-017: the id is public, the page needs it for One Tap).
pub async fn me(State(st): State<AppState>, viewer: Option<axum::Extension<Viewer>>) -> Response {
    let Some(auth) = st.auth.as_ref() else {
        return super::json_ok(&json!({"auth": false}));
    };
    let (user, admin) = match viewer {
        Some(axum::Extension(Viewer::Member(m))) => {
            let admin = m.admin;
            let user = m.user.clone();
            (Some(user), admin)
        }
        _ => (None, false),
    };
    super::json_ok(&json!({
        "auth": true,
        "user": user,
        "admin": admin,
        "client_id": auth.client_id(),
    }))
}

/// `DELETE /api/me`: delete the account; every table row of the user goes by cascade, and the
/// response clears the cookie. Signed out is 401 `sign-in required`. The cascade covers
/// `mt5_bridges` too (`user_id REFERENCES users(id) ON DELETE CASCADE`, migration 0003;
/// confirmed for CUJ 8 at 07-03): `store::delete_user` removes only the `users` row, so the
/// bridges need no explicit delete line.
pub async fn delete_me(State(st): State<AppState>, viewer: axum::Extension<Viewer>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match auth.delete_account(&viewer.0).await {
        Ok(cookies) => {
            let mut resp = StatusCode::NO_CONTENT.into_response();
            resp = cookies.attach(resp);
            resp
        }
        Err(e) => auth_error(e),
    }
}
