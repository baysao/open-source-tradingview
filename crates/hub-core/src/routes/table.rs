//! The route lists as data (03-ARCHITECTURE decision 2): one list per mode, `app()` mounts from
//! them and the gate tests enumerate them, so a route mounted outside the gate's areas fails a
//! test instead of opening a door.

use super::auth;
use super::logs;
use super::me;
use crate::AppState;
use axum::http::Method;
use axum::routing::MethodRouter;
use axum::routing::{get, post};
use axum::Router;

/// One mounted route, as the tests enumerate it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteSpec {
    pub method: Method,
    pub path: &'static str,
}

fn spec(
    method: Method,
    path: &'static str,
    r: MethodRouter<AppState>,
) -> (RouteSpec, MethodRouter<AppState>) {
    (RouteSpec { method, path }, r)
}

/// Routes in both modes: `/auth/me` (local: `{"auth": false}`).
pub(crate) fn shared() -> Vec<(RouteSpec, MethodRouter<AppState>)> {
    vec![spec(Method::GET, "/auth/me", get(me::me))]
}

/// Local-mode-only routes (Phase 1 parity, D-012): the log endpoints leave with auth on.
pub(crate) fn local() -> Vec<(RouteSpec, MethodRouter<AppState>)> {
    vec![
        spec(Method::GET, "/api/logs", get(logs::logs)),
        spec(Method::POST, "/api/logs/journal", post(logs::journal)),
    ]
}

/// Hosted-mode-only routes. Every one lies in a gate area: `/auth/`, `/admin/` or `/api/me`.
pub(crate) fn hosted() -> Vec<(RouteSpec, MethodRouter<AppState>)> {
    vec![
        spec(Method::POST, "/auth/google/onetap", post(auth::onetap)),
        spec(Method::GET, "/auth/google/start", get(auth::start)),
        spec(Method::GET, "/auth/google/callback", get(auth::callback)),
        spec(
            Method::POST,
            "/auth/desktop/exchange",
            post(auth::desktop_exchange),
        ),
        spec(Method::POST, "/auth/logout", post(auth::logout)),
        spec(
            Method::DELETE,
            "/api/me",
            axum::routing::delete(me::delete_me),
        ),
        // The Phase 1 handlers mounted directly behind the gate (no pass-through wrapper).
        spec(Method::GET, "/admin/logs", get(logs::logs)),
        spec(Method::POST, "/admin/logs/journal", post(logs::journal)),
    ]
}

/// What `app(state)` calls: routes from the lists with their paths, by mode.
pub(crate) fn mount_routes(state: &AppState) -> Router<AppState> {
    let mut router = Router::new();
    for (s, r) in shared().into_iter().chain(if state.auth.is_some() {
        hosted()
    } else {
        local()
    }) {
        router = router.route(s.path, r);
    }
    router
}

/// Every route `app(state)` serves, for the gate tests: shared, then the mode's own.
pub fn route_specs(state: &AppState) -> Vec<RouteSpec> {
    shared()
        .into_iter()
        .chain(if state.auth.is_some() {
            hosted()
        } else {
            local()
        })
        .map(|(s, _)| s)
        .collect()
}
