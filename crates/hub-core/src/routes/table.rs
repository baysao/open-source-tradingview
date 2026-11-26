//! The route lists as data (03-ARCHITECTURE decision 2): one list per mode, `app()` mounts from
//! them and the gate tests enumerate them, so a route mounted outside the gate's areas fails a
//! test instead of opening a door.

use super::auth;
use super::logs;
use super::me;
use super::metrics;
use super::mt5;
use super::pairing;
use super::state;
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

/// Routes in both modes: `/auth/me` (local: `{"auth": false}`) and the TTFC beacon, which is
/// anonymous and sits outside every gate area by design (`gate.rs`), so it guards itself.
pub(crate) fn shared() -> Vec<(RouteSpec, MethodRouter<AppState>)> {
    vec![
        spec(Method::GET, "/auth/me", get(me::me)),
        spec(Method::POST, "/api/metrics/ttfc", post(metrics::ttfc)),
    ]
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
        // The MT5 bridge surface (07-03, D-032's route table): owner-scoped CRUD + test, a
        // foreign id 404 (G5). Inside the gate area like every /api/me route (L-022).
        spec(Method::POST, "/api/me/mt5", post(mt5::register)),
        spec(Method::GET, "/api/me/mt5", get(mt5::list)),
        spec(Method::POST, "/api/me/mt5/{id}/test", post(mt5::test)),
        spec(
            Method::DELETE,
            "/api/me/mt5/{id}",
            axum::routing::delete(mt5::delete),
        ),
        // The pairing surface (09-01): the mint is session-gated like the block above it; pair
        // and revoke authenticate by a body secret and are the two paths the gate exempts from
        // its Origin rule (gate.rs `is_secret_authenticated`). Inside the gate area like every
        // /api/me route (L-022).
        spec(
            Method::POST,
            "/api/me/mt5/pairing-code",
            post(pairing::mint),
        ),
        spec(Method::POST, "/api/me/mt5/pair", post(pairing::pair)),
        spec(
            Method::POST,
            "/api/me/mt5/paired/{id}/revoke",
            post(pairing::revoke),
        ),
        // The saved-state surface (05-02): contract in routes/state.rs's module doc.
        spec(Method::GET, "/api/me/state", get(state::list)),
        spec(Method::GET, "/api/me/state/{key}", get(state::get_one)),
        spec(
            Method::PUT,
            "/api/me/state/{key}",
            axum::routing::put(state::put),
        ),
        spec(
            Method::DELETE,
            "/api/me/state/{key}",
            axum::routing::delete(state::delete),
        ),
        // The Phase 1 handlers mounted directly behind the gate (no pass-through wrapper).
        spec(Method::GET, "/admin/logs", get(logs::logs)),
        spec(Method::POST, "/admin/logs/journal", post(logs::journal)),
        spec(Method::GET, "/admin/metrics", get(metrics::admin_metrics)),
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
