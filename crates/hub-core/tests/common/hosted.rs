//! The hosted harness (03-02 suites): a real engine (pgsqlite, or Postgres in CI), the
//! in-memory test Idp, the manual clock, and the router served on a loopback socket. Every
//! flow runs through `app(state)` over HTTP (03-ARCHITECTURE decision 7: tests run every
//! flow through the app on both engines, L-016: no paused clock with sockets - the clock is
//! manual, the database is a real process).

#![allow(dead_code)] // each suite uses a part

use hub_core::auth::test_idp::{Claims, ManualClock, TestIdp};
use hub_core::config::Config;
use hub_core::db::Db;
use hub_core::{app, AppState};

use axum::Router;
use reqwest::redirect::Policy;
use reqwest::Client;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;

#[path = "db.rs"]
pub mod db_common;

/// 32 bytes of 0x01, base64url: the test `VTV_SECRET_KEY` (as in the config tests).
pub const KEY32: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE";

/// The test Google client id (`GOOGLE_CLIENT_ID`).
pub const CLIENT_ID: &str = "test-client-id";

/// The hosted app under test. Dropping it closes the server task (the JoinHandle) and the
/// pool; the engine process belongs to the [`db_common::TestDb`] the test owns.
pub struct Harness {
    pub app: Router,
    pub state: AppState,
    pub idp: TestIdp,
    pub clock: ManualClock,
    pub client: Client,
    /// `PUBLIC_ORIGIN`: the only Origin a write may carry (test-idp builds accept loopback http).
    pub origin: String,
}

impl Harness {
    /// Builds the hosted state over `db` (opened and migrated), the in-memory Idp and the
    /// manual clock. `admin_emails` is the `ADMIN_EMAILS` list; `origin` must be
    /// `http://127.0.0.1:<port>` (test-idp) or `https://...`.
    pub async fn hosted(
        db: &db_common::TestDb,
        admin_emails: &[&str],
        origin: &str,
        idp_kind: IdpKind,
    ) -> Harness {
        let idp = TestIdp::new();
        let clock = ManualClock::new(1_700_000_000);
        // AuthSettings comes from the real parser: `from_lookup` with every hosted variable
        // set (the test-idp feature accepts a loopback http origin).
        if matches!(idp_kind, IdpKind::Http) {
            unreachable!("the HTTP Idp is built by `over_http` after the bind");
        }
        let database_url = db.url.clone();
        let client_id = CLIENT_ID.to_string();
        let key32 = KEY32.to_string();
        let admins: Vec<String> = admin_emails.iter().map(|s| s.to_string()).collect();
        let cfg = Config::from_lookup(move |k| match k {
            "AUTH_ENABLED" => Some("true".to_string()),
            "DATABASE_URL" => Some(database_url.clone()),
            "GOOGLE_CLIENT_ID" => Some(client_id.clone()),
            "GOOGLE_CLIENT_SECRET" => Some("test-client-secret".to_string()),
            "VTV_SECRET_KEY" => Some(key32.clone()),
            "PUBLIC_ORIGIN" => Some(origin.to_string()),
            "ADMIN_EMAILS" => Some(admins.join(",")),
            "HUB_STATIC_DIR" => Some("hub".to_string()),
            _ => None,
        })
        .expect("hosted test config");
        let settings = match &cfg.auth {
            hub_core::config::AuthMode::Google(s) => s.clone(),
            hub_core::config::AuthMode::Off => unreachable!("hosted config"),
        };
        let deps = hub_core::auth::AuthDeps::in_memory(&idp, &clock);
        let auth = hub_core::auth::Auth::new(
            settings.clone(),
            origin,
            Db::open(&db.secret()).await.expect("open and migrate"),
            deps,
        );
        let state = AppState::build_with_auth(cfg, Default::default(), auth);
        let client = Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("http client");
        Harness {
            app: app(state.clone()),
            state,
            idp,
            clock,
            client,
            origin: origin.to_string(),
        }
    }

    /// The in-memory Idp with the HTTP Idp over a loopback socket: the app's own
    /// `/__test-idp/*` routes serve as Google (JWKS with `Cache-Control`, the token POST),
    /// so PKCE and the verifier round trip over real HTTP. The app is rebuilt with the
    /// endpoints pointed at itself, then served.
    pub async fn over_http(
        db: &db_common::TestDb,
        admin_emails: &[&str],
    ) -> (Harness, String, tokio::task::JoinHandle<()>) {
        // First build with the memory Idp (its router is what answers /__test-idp/*), learn
        // the port, then rebuild with the HTTP Idp pointed at it. The Idp handle is shared.
        let probe = Self::hosted(db, admin_emails, "http://127.0.0.1:8770", IdpKind::Memory).await;
        let idp = probe.idp.clone();
        let clock = probe.clock.clone();
        drop(probe);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = format!("http://127.0.0.1:{port}");

        let (cfg, settings) = hosted_cfg_with(
            db,
            admin_emails,
            &origin,
            [
                ("GOOGLE_AUTH_URL", format!("{origin}/__test-idp/authorize")),
                ("GOOGLE_TOKEN_URL", format!("{origin}/__test-idp/token")),
                ("GOOGLE_JWKS_URL", format!("{origin}/__test-idp/jwks")),
            ],
        );
        let auth = hub_core::auth::Auth::new(
            settings.clone(),
            &origin,
            Db::open(&db.secret()).await.expect("open and migrate"),
            hub_core::auth::AuthDeps::over_http(&settings, &clock, &origin),
        );
        let state = AppState::build_with_auth(cfg, Default::default(), auth);
        // The app's own router (its /auth routes and gate included) with the IdP router
        // nested as a service - the same shape lib.rs mounts for a memory Idp.
        let app = app(state.clone()).nest_service(
            "/__test-idp",
            hub_core::auth::test_idp::router(idp.clone(), CLIENT_ID.to_string()),
        );
        let client = Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("http client");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("serve");
        });
        (
            Harness {
                app: Router::new(), // requests go over the socket, not through the router
                state,
                idp,
                clock,
                client,
                origin: origin.clone(),
            },
            origin,
            server,
        )
    }

    /// Serves the app on an ephemeral port; returns the base URL and the server guard.
    pub async fn serve(&self) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = self.app.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("serve");
        });
        (format!("http://127.0.0.1:{port}"), server)
    }
}

/// Which Idp the harness's Auth talks to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IdpKind {
    /// The in-process TestIdp (no socket for the IdP; L-016's fast path).
    Memory,
    /// The HTTP Idp against the served `/__test-idp/*` router (AC-3's verifier check).
    Http,
}

/// The hosted config and its settings, from the real parser (`from_lookup`). The `GOOGLE_*`
/// variables are fed through so a test can point the Idp at its own `/__test-idp` socket
/// (the overrides exist only under the `test-idp` feature, which these suites run with).
fn hosted_cfg(
    db: &db_common::TestDb,
    admin_emails: &[&str],
    origin: &str,
) -> (Config, Arc<hub_core::config::AuthSettings>) {
    hosted_cfg_with(db, admin_emails, origin, std::iter::empty())
}

/// [`hosted_cfg`] with extra variables (the `GOOGLE_*_URL` endpoint overrides).
fn hosted_cfg_with(
    db: &db_common::TestDb,
    admin_emails: &[&str],
    origin: &str,
    extra: impl IntoIterator<Item = (&'static str, String)>,
) -> (Config, Arc<hub_core::config::AuthSettings>) {
    let database_url = db.url.clone();
    let client_id = CLIENT_ID.to_string();
    let key32 = KEY32.to_string();
    let admins: Vec<String> = admin_emails.iter().map(|s| s.to_string()).collect();
    let extra: std::collections::HashMap<&'static str, String> = extra.into_iter().collect();
    let cfg = Config::from_lookup(move |k| {
        if let Some(v) = extra.get(k) {
            return Some(v.clone());
        }
        match k {
            "AUTH_ENABLED" => Some("true".to_string()),
            "DATABASE_URL" => Some(database_url.clone()),
            "GOOGLE_CLIENT_ID" => Some(client_id.clone()),
            "GOOGLE_CLIENT_SECRET" => Some("test-client-secret".to_string()),
            "VTV_SECRET_KEY" => Some(key32.clone()),
            "PUBLIC_ORIGIN" => Some(origin.to_string()),
            "ADMIN_EMAILS" => Some(admins.join(",")),
            "HUB_STATIC_DIR" => Some("hub".to_string()),
            _ => None,
        }
    })
    .expect("hosted test config");
    let settings = match &cfg.auth {
        hub_core::config::AuthMode::Google(s) => s.clone(),
        hub_core::config::AuthMode::Off => unreachable!("hosted config"),
    };
    (cfg, settings)
}

/// A layer that forwards every event to every registered ring.
///
/// A tracing global subscriber installs once per process and `set_default` is thread-local
/// (measured: a request served on another tokio thread never reached the ring, which held
/// only the boot line). This layer keeps the test harnesses' [`LogBus`]es and pushes each
/// event into each, so a test that scans a ring sees its own app's lines whatever order the
/// tests ran in.
#[cfg(feature = "test-idp")]
#[derive(Default)]
struct FanOut {
    buses: std::sync::Mutex<Vec<hub_core::logbus::LogBus>>,
}

/// Every field of an event, formatted the way the log panel would show it: the message plus
/// any `key=value` facts. A secret logged as a separate field must appear here, or the scan
/// would miss exactly the leak it exists to catch (found by the AC-8 pass).
#[cfg(feature = "test-idp")]
#[derive(Default)]
struct Msg(String);

#[cfg(feature = "test-idp")]
impl tracing::field::Visit for Msg {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        // `format_args!` prints through Debug without quotes, so the text is verbatim.
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.0 = text;
        } else {
            self.0.push_str(&format!(" {}={text}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        } else {
            self.0.push_str(&format!(" {}={value}", field.name()));
        }
    }
}

#[cfg(feature = "test-idp")]
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for &'static FanOut {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        // The same visibility rule RingLayer applies: feed/server at any level, others WARN+.
        if meta.target() != "feed"
            && meta.target() != "server"
            && *meta.level() > tracing::Level::WARN
        {
            return;
        }
        let mut msg = Msg(String::new());
        event.record(&mut msg);
        let lvl = match *meta.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            _ => "debug",
        };
        let src = hub_core::logbus::source_of(meta.target(), None);
        for bus in self.buses.lock().unwrap_or_else(|p| p.into_inner()).iter() {
            bus.push(hub_core::logbus::Line::new(src.clone(), lvl, msg.0.clone()));
        }
    }
}

#[cfg(feature = "test-idp")]
fn fan_out() -> &'static FanOut {
    use std::sync::OnceLock;
    static LAYER: OnceLock<&'static FanOut> = OnceLock::new();
    LAYER.get_or_init(|| {
        use tracing_subscriber::prelude::*;
        let layer: &'static FanOut = Box::leak(Box::new(FanOut::default()));
        // WARN events (the refusal lines) only reach a layer when the subscriber's max level
        // admits them; without this the ring stays empty and the scan proves nothing.
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry()
                .with(tracing_subscriber::filter::LevelFilter::DEBUG)
                .with(layer),
        );
        layer
    })
}

/// Registers `h`'s ring with the process-wide fan-out and returns the bus to scan.
#[cfg(feature = "test-idp")]
pub fn feed_ring(h: &Harness) -> hub_core::logbus::LogBus {
    let bus = h.state.logs.clone();
    let layer = fan_out();
    layer
        .buses
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(bus.clone());
    bus
}

/// A valid One Tap credential for `sub` (audience ours, verified email). The expiry is a
/// day after the harness's clock, so the manual clock can advance during a test without
/// expiring the token it mints (a real Idp answers tokens valid for minutes to an hour).
pub fn one_tap_token(h: &Harness, sub: &str) -> String {
    let exp = h.clock.now().timestamp() + 24 * 3600;
    h.idp.id_token(
        &Claims::for_sub(sub).aud(CLIENT_ID).exp(Some(Some(exp))),
        None,
        0,
    )
}

/// POST JSON with the Origin header through the served app.
pub async fn post_json(base: &str, h: &Harness, path: &str, body: Value) -> reqwest::Response {
    h.client
        .post(format!("{base}{path}"))
        .header("Origin", &h.origin)
        .json(&body)
        .send()
        .await
        .expect("POST")
}

/// GET through the served app.
pub async fn get(base: &str, h: &Harness, path: &str) -> reqwest::Response {
    h.client
        .get(format!("{base}{path}"))
        .send()
        .await
        .expect("GET")
}

/// GET with an explicit Origin header.
pub async fn get_origin(
    base: &str,
    h: &Harness,
    path: &str,
    origin: Option<&str>,
) -> reqwest::Response {
    let mut req = h.client.get(format!("{base}{path}"));
    if let Some(o) = origin {
        req = req.header("Origin", o);
    }
    req.send().await.expect("GET")
}

/// POST with a raw body and content type (the 415 and 413 cases).
pub async fn post_raw(
    base: &str,
    h: &Harness,
    path: &str,
    ctype: &str,
    body: impl Into<Vec<u8>>,
) -> reqwest::Response {
    h.client
        .post(format!("{base}{path}"))
        .header("Origin", &h.origin)
        .header("Content-Type", ctype)
        .body(body.into())
        .send()
        .await
        .expect("POST")
}

/// The value of one Set-Cookie attribute string, e.g. the session cookie's value.
pub fn cookie_value(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            let rest = s.strip_prefix(name)?;
            let value = rest.strip_prefix('=')?;
            Some(value.split(';').next().unwrap_or("").to_string())
        })
}

/// All Set-Cookie header strings.
pub fn set_cookies(resp: &reqwest::Response) -> Vec<String> {
    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect()
}

/// The Cookie header a response's session cookie produces (the browser's next request).
pub fn cookie_header(resp: &reqwest::Response, name: &str) -> Option<String> {
    cookie_value(resp, name).map(|v| format!("{name}={v}"))
}

/// Wait for a served port to accept connections.
pub async fn wait_port(addr: SocketAddr) -> bool {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    false
}

/// Sign in over One Tap and return the response (200 expected by the caller).
pub async fn sign_in(base: &str, h: &Harness, sub: &str) -> reqwest::Response {
    let token = one_tap_token(h, sub);
    post_json(
        base,
        h,
        "/auth/google/onetap",
        serde_json::json!({"credential": token}),
    )
    .await
}

/// The JSON body of a response.
pub async fn body_json(resp: reqwest::Response) -> Value {
    resp.json().await.expect("json body")
}
