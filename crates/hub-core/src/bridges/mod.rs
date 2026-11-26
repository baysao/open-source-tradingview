//! Per-user MT5 bridges (Phase 07, 07-ARCHITECTURE): the SSRF guard, the token crypto, the
//! outbound client and the store. The routes that use them live in `crate::routes::mt5` (07-03);
//! nothing here touches a route, the catalog or the feeds.
//!
//! The one new dependency of this phase is `chacha20poly1305`; everything else was already in the
//! tree (reqwest carries the URL parser, `hmac`/`sha2` derive the key, `getrandom` mints nonces).

pub mod client;
pub mod crypto;
pub mod guard;
pub mod store;

use crate::config::LocalBridge;
use std::sync::Arc;

/// The semaphore key the local bridge's client calls use: there is no row for `local`, so the
/// per-bridge permit pool keys on this constant (the env bridge is the only one in local mode).
pub const LOCAL_SEMAPHORE_ID: uuid::Uuid = uuid::Uuid::nil();

/// The bridge services [`AppState`](crate::AppState) carries (07-ARCHITECTURE "Shape"): the
/// outbound client every route and poller shares, and the local-mode bridge (D-033) when local
/// mode configured one. `Clone` like every `AppState` field; a clone shares the one client
/// through the `Arc`, so the pinned-client cache and the per-bridge semaphores stay per-process.
#[derive(Clone)]
pub struct Bridges {
    pub client: Arc<client::BridgeClient>,
    /// `VTV_LOCAL_BRIDGE_URL` + `VTV_LOCAL_BRIDGE_TOKEN` (D-033), local mode only. The URL is
    /// re-vetted with `auth_enabled = false` on every call, so the loopback exception bites at
    /// call time, not just at boot.
    pub local: Option<LocalBridge>,
}
