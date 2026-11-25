//! `/api/me/state` — the saved-state HTTP surface (05-02; storage from 05-01).
//!
//! The wire contract this module ships (05-02-PLAN "The wire contract this plan fixes";
//! 05-03 codes against exactly this):
//!
//! | Call | Request | Success | Failure |
//! |---|---|---|---|
//! | `GET /api/me/state` | — | 200 `{"keys":[{"key","version","updated_at"}]}` | 401 |
//! | `GET /api/me/state/{key}` | — | 200 `{"key","value","version","updated_at"}` | 401; 404 `not found` |
//! | `PUT /api/me/state/{key}` | body `{"value":<string>}`, header `If-Match: <n>` | 200 `{"key","version"}` | 409 `{"error":"version conflict","version","value"}`; 413 `value too large`; 409 `too many keys`; 400 `bad key`; 428 `if-match required`; 400 `bad if-match`; 401 |
//! | `DELETE /api/me/state/{key}` | — | 204 | 401 |
//!
//! `expected == 0` means the key must not exist yet (an insert). The 409 body carries the
//! server's current value so the client can offer the other device's version without a second
//! request (CUJ 5). The list deliberately omits values: it is the merge/status snapshot, and
//! shipping every value would make it the largest response in the app.

use super::auth::auth_error;
use crate::auth::{PutOutcome, StateRow, Viewer};
use crate::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The key charset: 1 to 128 bytes of `A-Za-z0-9` or one of `_-.:`. `:` is required because
/// `backup:<key>:<date>` keys are written through this same path during a merge. `/` would
/// collide with the path segment; whitespace never appears in a written key.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// `GET /api/me/state`: the user's key list without values (the merge snapshot).
pub async fn list(State(st): State<AppState>, viewer: axum::Extension<Viewer>) -> Response {
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let m = match &viewer.0 {
        Viewer::Member(m) => m,
        _ => return auth_error(crate::auth::AuthError::SignInRequired),
    };
    match auth.state_all(m).await {
        Ok(rows) => super::json_ok(&KeysBody {
            keys: rows
                .into_iter()
                .map(|r| KeyEntry {
                    key: r.key,
                    version: r.version,
                    updated_at: r.updated_at,
                })
                .collect(),
        }),
        Err(e) => auth_error(e),
    }
}

#[derive(Serialize)]
struct KeysBody {
    keys: Vec<KeyEntry>,
}

#[derive(Serialize)]
struct KeyEntry {
    key: String,
    version: i32,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// `GET /api/me/state/{key}`: one row with its value, or 404 `not found`.
pub async fn get_one(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    Path(key): Path<String>,
) -> Response {
    // Sign-in before the key check, as `put` does: AC-6 says a signed-out caller gets 401 on
    // every route, whatever the input. A bad key answered first would make that input-dependent.
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let m = match &viewer.0 {
        Viewer::Member(m) => m,
        _ => return auth_error(crate::auth::AuthError::SignInRequired),
    };
    if !valid_key(&key) {
        return super::api_error(StatusCode::NOT_FOUND, "not found");
    }
    match auth.state_all(m).await {
        Ok(rows) => match rows.iter().find(|r| r.key == key) {
            Some(row) => super::json_ok(&StateRowRef(row)),
            None => super::api_error(StatusCode::NOT_FOUND, "not found"),
        },
        Err(e) => auth_error(e),
    }
}

/// `PUT /api/me/state/{key}`: optimistic-concurrency write. `If-Match` is the version the
/// client last saw; 0 inserts. Refusals in the order the client distinguishes them:
/// 401 sign-in, 400 bad key, 428/400 If-Match, then the storage outcome mapped 1:1.
pub async fn put(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    headers: HeaderMap,
    Path(key): Path<String>,
    body: axum::Json<PutBody>,
) -> Response {
    // Sign-in first: the gate resolves the viewer after its own Origin/body rules, so a
    // signed-out PUT must answer 401 whatever its If-Match says (AC-6's order).
    let m = match &viewer.0 {
        Viewer::Member(m) => m.clone(),
        _ => return auth_error(crate::auth::AuthError::SignInRequired),
    };
    if !valid_key(&key) {
        return super::api_error(StatusCode::BAD_REQUEST, "bad key");
    }
    let expected = match parse_if_match(headers.get("If-Match")) {
        Ok(n) => n,
        Err(IfMatchError::Missing) => {
            return super::api_error(StatusCode::PRECONDITION_REQUIRED, "if-match required")
        }
        Err(IfMatchError::Malformed) => {
            return super::api_error(StatusCode::BAD_REQUEST, "bad if-match")
        }
    };
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    match auth.state_put(&m, &key, &body.value, expected).await {
        Ok(PutOutcome::Stored { version }) => super::json_ok(&PutOk { key, version }),
        Ok(PutOutcome::Conflict { version, value }) => super::json(
            StatusCode::CONFLICT,
            &ConflictBody {
                error: "version conflict",
                version,
                value,
            },
        ),
        Ok(PutOutcome::TooLarge) => {
            super::api_error(StatusCode::PAYLOAD_TOO_LARGE, "value too large")
        }
        Ok(PutOutcome::TooManyKeys) => super::api_error(StatusCode::CONFLICT, "too many keys"),
        Err(e) => auth_error(e),
    }
}

#[derive(serde::Deserialize)]
pub struct PutBody {
    value: String,
}

#[derive(Serialize)]
struct PutOk {
    key: String,
    version: i32,
}

/// The 409 body: the current version and value, so the client can adopt the other device's
/// write without a second request (CUJ 5). A struct, not `json!`, so the field names the
/// 05-03 client reads are compile-checked here.
#[derive(Serialize)]
struct ConflictBody<'a> {
    error: &'a str,
    version: i32,
    value: String,
}

enum IfMatchError {
    Missing,
    Malformed,
}

/// `If-Match: <n>`: trimmed, quote- and `W/`-stripped decimal integer (RFC 9110 §13.1.1
/// validators; the weak prefix cannot occur on a version the client got from us, but strip
/// it before parsing anyway so the error is `bad if-match`, not a panic).
fn parse_if_match(v: Option<&axum::http::HeaderValue>) -> Result<i32, IfMatchError> {
    let raw = v.ok_or(IfMatchError::Missing)?;
    let s = raw.to_str().map_err(|_| IfMatchError::Malformed)?;
    let s = s.trim();
    let s = s.strip_prefix("W/").unwrap_or(s);
    let s = s.trim_matches('"');
    s.parse::<i32>().map_err(|_| IfMatchError::Malformed)
}

/// `DELETE /api/me/state/{key}`: remove one key; always 204 (a missing key is success —
/// `removeItem` is not an error, and CUJ 4 must not wedge on it).
pub async fn delete(
    State(st): State<AppState>,
    viewer: axum::Extension<Viewer>,
    Path(key): Path<String>,
) -> Response {
    // Sign-in first (AC-6): a signed-out DELETE is 401, never a silent 204.
    let auth = st.auth.as_ref().expect("mounted only when hosted");
    let m = match &viewer.0 {
        Viewer::Member(m) => m,
        _ => return auth_error(crate::auth::AuthError::SignInRequired),
    };
    if !valid_key(&key) {
        return StatusCode::NO_CONTENT.into_response();
    }
    match auth.state_delete(m, &key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => auth_error(e),
    }
}

/// A `StateRow` serialises as the contract's get_one body (field names compile-checked).
impl Serialize for StateRowRef<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("StateRow", 4)?;
        st.serialize_field("key", &self.0.key)?;
        st.serialize_field("value", &self.0.value)?;
        st.serialize_field("version", &self.0.version)?;
        st.serialize_field("updated_at", &self.0.updated_at)?;
        st.end()
    }
}

struct StateRowRef<'a>(&'a StateRow);
