//! The gate table (03-02 AC-5): every hosted non-GET route from `route_specs` is sent with no
//! Origin, with a foreign Origin and with PUBLIC_ORIGIN; GET `/auth/me` needs no Origin; every
//! hosted route lies in a gate area; text/plain is 415 and 300,001 bytes are 413. Plus AC-7's
//! mode table: local answers `{"auth": false}` and 404s the hosted routes; hosted 404s
//! `/api/logs*`.
//!
//! One route is outside every area on purpose (04-02): `POST /api/metrics/ttfc`. It is anonymous,
//! so it lives in `shared()` and guards itself with `net::post_guard`, which allows a missing
//! Origin (curl parity) and refuses a foreign one. [`THE_BEACON`] is the single exception the two
//! table loops below skip, and the loop still proves the exception is the only one.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{body_json, db_common, get, post_raw, IdpKind};
use serde_json::json;

const PUBLIC: &str = "http://127.0.0.1:8770";

/// The one route mounted outside the gate, and why: an anonymous beacon that guards itself.
const THE_BEACON: &str = "/api/metrics/ttfc";

engines!(
    fn every_hosted_route_is_in_a_gate_area(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let specs = hub_core::routes::table::route_specs(&h.state);
        assert!(!specs.is_empty(), "the hosted table is mounted");
        let mut outside = Vec::new();
        for s in &specs {
            let in_area = s.path.starts_with("/auth/")
                || s.path.starts_with("/admin/")
                || s.path == "/api/me"
                || s.path.starts_with("/api/me/");
            if !in_area {
                outside.push(s.path);
            }
            assert!(
                in_area || s.path == THE_BEACON,
                "{} {} lies outside every gate area",
                s.method,
                s.path
            );
        }
        // Exactly one outside route, and it is the beacon: a second ungated route fails here.
        assert_eq!(outside, vec![THE_BEACON], "only the beacon is ungated");
        // The area decision itself is unit-tested in `src/routes/gate.rs`; here the mounted
        // table is the contract.
    }
);

engines!(
    fn origin_rules_over_every_hosted_non_get(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let specs = hub_core::routes::table::route_specs(&h.state);

        for s in &specs {
            if s.method == reqwest::Method::GET {
                continue;
            }
            // The beacon is outside the gate: no Origin is allowed (curl), and its own guard
            // decides a foreign one. Both are asserted once, below the loop.
            if s.path == THE_BEACON {
                continue;
            }
            let url = format!("{base}{}", s.path);
            // No Origin: refused.
            let method = s.method.clone();
            let r = match method {
                reqwest::Method::POST => h.client.post(&url),
                reqwest::Method::DELETE => h.client.delete(&url),
                m => panic!("unexpected method {m}"),
            }
            .json(&json!({}))
            .send()
            .await
            .unwrap();
            // `/admin/*` decides admin-or-404 before the Origin rule (an outsider learns
            // nothing about which admin routes exist); every other area is 403.
            let expected = if s.path.starts_with("/admin/") { 404 } else { 403 };
            assert_eq!(r.status(), expected, "{} {} without Origin", method, s.path);

            // Foreign Origin: refused.
            let r = match s.method {
                reqwest::Method::POST => h.client.post(&url),
                reqwest::Method::DELETE => h.client.delete(&url),
                _ => unreachable!(),
            }
            .header("Origin", "https://evil.example")
            .json(&json!({}))
            .send()
            .await
            .unwrap();
            let expected = if s.path.starts_with("/admin/") { 404 } else { 403 };
            assert_eq!(r.status(), expected, "{} {} foreign Origin", s.method, s.path);

            // PUBLIC_ORIGIN: reaches the handler (401/400/404/204 by the route's own rules,
            // never 403).
            let r = match s.method {
                reqwest::Method::POST => h.client.post(&url),
                reqwest::Method::DELETE => h.client.delete(&url),
                _ => unreachable!(),
            }
            .header("Origin", PUBLIC)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
            assert_ne!(r.status(), 403, "{} {} with PUBLIC_ORIGIN", s.method, s.path);
        }
        server.abort();
    }
);

engines!(
    fn the_beacon_guards_itself_outside_the_gate(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let url = format!("{base}{THE_BEACON}");
        let good = json!({"ms": 1234, "engine": "vela"});

        // No Origin (curl) reaches the handler: the beacon is anonymous by design.
        let r = h.client.post(&url).json(&good).send().await.unwrap();
        assert_eq!(r.status(), 204, "no Origin, valid body");

        // A foreign Origin is refused by the guard the beacon applies itself.
        let r = h
            .client
            .post(&url)
            .header("Origin", "https://evil.example")
            .json(&good)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "foreign Origin");

        // post_guard's media-type rule, then this route's own caps.
        let r = post_raw(&base, &h, THE_BEACON, "text/plain", b"{}".to_vec()).await;
        assert_eq!(r.status(), 415, "text/plain");
        let over_guard = format!("\"{}\"", "a".repeat(300_000));
        let r = post_raw(&base, &h, THE_BEACON, "application/json", over_guard.into_bytes()).await;
        assert_eq!(r.status(), 413, "over post_guard's 300,000");
        let over_route = format!("{{\"ms\": 1000, \"engine\": \"vela\", \"pad\": \"{}\"}}", "a".repeat(1000));
        let r = post_raw(&base, &h, THE_BEACON, "application/json", over_route.into_bytes()).await;
        assert_eq!(r.status(), 413, "over the beacon's own 1000 bytes");

        // The two field rules: the range, then the engine name.
        for body in [
            json!({"ms": 120_001, "engine": "vela"}),
            json!({"ms": 1000, "engine": "pine"}),
            json!({"ms": 1000}),
            json!({"ms": -1, "engine": "vela"}),
            json!({"ms": 1000, "engine": "vela", "extra": 1}),
        ] {
            let r = h.client.post(&url).json(&body).send().await.unwrap();
            assert_eq!(r.status(), 400, "{body}");
        }
        // The last edge is accepted (the boundary is inclusive).
        let r = h
            .client
            .post(&url)
            .json(&json!({"ms": 120_000, "engine": "klinechart"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "the last edge is in range");

        // The read is admin-only: an anonymous GET learns nothing (404 before the Origin rule).
        let r = get(&base, &h, "/admin/metrics").await;
        assert_eq!(r.status(), 404, "anonymous /admin/metrics");
        server.abort();
    }
);

// AC-2's 429 at the wire, not just in the limiter's unit test: the status, the `Retry-After`
// header and the `api_error` body shape are what a client actually meets, and a middleware wired
// to the wrong bucket would still pass a unit test of `take`.
//
// The exact thresholds stay in `src/net.rs`, where the tests drive a manual `Instant`. Here the
// loop is "call until refused" rather than "the 11th call fails", because a real clock refills
// tokens between calls: on a slow machine the burst can outlast 10 attempts, and asserting the
// exact index would make the test fail for being slow instead of for being wrong. What is
// asserted exactly is the part the clock cannot race: at least the whole burst got through
// first, and a refusal at 5/s always asks for 1 second.
engines!(
    fn a_spent_bucket_answers_429_with_retry_after(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        let mut allowed = 0;
        let mut refused = None;
        for _ in 0..30 {
            let r = get(&base, &h, "/auth/me").await;
            if r.status() == 429 {
                refused = Some(r);
                break;
            }
            assert_eq!(r.status(), 200, "/auth/me before the bucket is spent");
            allowed += 1;
        }
        let r = refused.expect("the Auth bucket answers 429 within 30 calls");
        assert!(allowed >= 10, "the burst of 10 got through first: {allowed}");
        let retry = r
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let ctype = r
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // At 5/s the wait for one token is under 0.2 s, so it always rounds up to 1 second.
        assert_eq!(retry, "1", "Retry-After names the wait in seconds");
        assert_eq!(ctype, "application/json", "the 429 is a JSON error");
        let body = body_json(r).await;
        assert_eq!(body["error"], "too many requests", "the api_error shape");

        // The drained Auth bucket never slows the Api one: the beacon is a different bucket and
        // still answers 204 while the auth route is refusing.
        let r = h
            .client
            .post(format!("{base}{THE_BEACON}"))
            .json(&json!({"ms": 1000, "engine": "vela"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204, "a spent Auth bucket leaves Api alone");
        server.abort();
    }
);

engines!(
    fn get_auth_me_needs_no_origin_and_body_rules_apply(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        // GET /auth/me needs no Origin and answers the hosted shape.
        let r = get(&base, &h, "/auth/me").await;
        assert_eq!(r.status(), 200);
        let me = body_json(r).await;
        assert_eq!(me["auth"], true);
        assert_eq!(me["user"], serde_json::Value::Null);
        assert_eq!(me["client_id"], hosted::CLIENT_ID);

        // text/plain body: 415.
        let r = post_raw(&base, &h, "/auth/google/onetap", "text/plain", b"hello").await;
        assert_eq!(r.status(), 415, "text/plain body");

        // 300,001 bytes of JSON: 413.
        let big = format!("\"{}\"", "a".repeat(300_000));
        let r = post_raw(&base, &h, "/auth/google/onetap", "application/json", big.into_bytes()).await;
        assert_eq!(r.status(), 413, "300,001-byte body");

        // Just under the limit is not 413 (400 unparsable, or 401 credential missing).
        let ok_size = format!("\"{}\"", "a".repeat(299_000));
        let r = post_raw(&base, &h, "/auth/google/onetap", "application/json", ok_size.into_bytes()).await;
        assert_ne!(r.status(), 413);
        assert_ne!(r.status(), 415);
        server.abort();
    }
);

engines!(
    fn local_and_hosted_modes_answer_the_shape_table(db) {
        // Local mode: the Phase 1 behaviour is unchanged.
        let cfg = hub_core::config::Config::from_lookup(|k| match k {
            "HUB_HTTP_PORT" => Some("8770".to_string()),
            "HUB_STATIC_DIR" => Some("hub".to_string()),
            _ => None,
        })
        .unwrap();
        let state = hub_core::AppState::build_with(cfg, Default::default());
        assert!(state.auth.is_none());
        let h_local = LocalApp::new(hub_core::app(state));
        let (base, server) = h_local.serve().await;

        let r = h_local.get(&base, "/auth/me").await;
        assert_eq!(r.status(), 200);
        assert_eq!(body_json(r).await, json!({"auth": false}), "local /auth/me");

        // The hosted routes are 404 JSON in local mode.
        let r = h_local.get(&base, "/auth/google/start").await;
        assert_eq!(r.status(), 404);
        assert_eq!(body_json(r).await["error"], "not found");
        let r = h_local.get(&base, "/admin/logs").await;
        assert_eq!(r.status(), 404);
        let r = h_local.client.delete(format!("{base}/api/me")).send().await.unwrap();
        assert_eq!(r.status(), 404);

        // Phase 1 parity: /api/logs and /api/logs/journal still answer in local mode.
        let r = h_local.get(&base, "/api/logs").await;
        assert_eq!(r.status(), 200, "local /api/logs");
        let r = h_local
            .client
            .post(format!("{base}/api/logs/journal"))
            .json(&json!({"lines": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "local /api/logs/journal");

        // Unknown /auth/ and /admin/ paths are JSON 404 (the fallback's prefixes).
        let r = h_local.get(&base, "/auth/nothing").await;
        assert_eq!(r.status(), 404);
        assert_eq!(body_json(r).await["error"], "not found");
        let r = h_local.get(&base, "/admin/nothing").await;
        assert_eq!(r.status(), 404);
        server.abort();

        // Hosted mode: /api/logs* answer 404 (they left with auth on, D-012).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = get(&base, &h, "/api/logs").await;
        assert_eq!(r.status(), 404, "hosted /api/logs");
        let r = h
            .client
            .post(format!("{base}/api/logs/journal"))
            .header("Origin", PUBLIC)
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "hosted /api/logs/journal");

        // Hosted: unknown /auth/* and /admin/* are 404 JSON from the fallback.
        let r = get(&base, &h, "/admin/nothing").await;
        assert_eq!(r.status(), 404);
        assert_eq!(body_json(r).await["error"], "not found");
        let r = get(&base, &h, "/auth/nothing").await;
        assert_eq!(r.status(), 404);
        server.abort();
    }
);

/// AC-1's header clause at the wire: each page's response carries `CspSet::scan`'s policy for that
/// path, and everything else carries the strict default.
///
/// The static root is a temp dir (L-008: tests never write into the repo), because the assertion is
/// about selection, not about the shipped pages: a fixture page with a known inline block pins the
/// hash, and `index.html` is the page with none. The real pages' own policies are proven in the
/// browser by `.paul/runs/check-0402.mjs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_page_carries_its_own_csp_and_the_rest_carry_the_default() {
    const BODY: &str = "console.log(\"csp-0402\");";
    // sha256 of BODY, computed independently: a policy that dropped the hashing would still
    // contain a `sha256-` token, so the value is pinned rather than pattern-matched.
    const HASH: &str = "sha256-yn7K6MlscoBW9r/X4XY4WjOecWg+v4YMDBo2HoU4kzM=";

    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("hashed.html"),
        format!("<!doctype html><html><body><script>{BODY}</script></body></html>"),
    )
    .expect("write fixture page");
    // No inline block: the default policy, not a per-page entry.
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><body><p>no scripts</p></body></html>",
    )
    .expect("write index");

    let static_dir = dir.path().to_string_lossy().to_string();
    let cfg = hub_core::config::Config::from_lookup(move |k| match k {
        "HUB_HTTP_PORT" => Some("8770".to_string()),
        "HUB_STATIC_DIR" => Some(static_dir.clone()),
        _ => None,
    })
    .unwrap();
    let state = hub_core::AppState::build_with(cfg, Default::default());
    let default_csp = state.csp.default().to_str().unwrap().to_string();
    let app = LocalApp::new(hub_core::app(state));
    let (base, server) = app.serve().await;

    let csp_of = |r: &reqwest::Response| {
        r.headers()
            .get("content-security-policy")
            .expect("every response carries a CSP")
            .to_str()
            .unwrap()
            .to_string()
    };

    let r = app.get(&base, "/hashed.html").await;
    assert_eq!(r.status(), 200);
    let page = csp_of(&r);
    assert!(
        page.contains(HASH),
        "the served page's policy must name its inline block's hash: {page}"
    );
    assert!(
        page.contains("worker-src 'self' blob:"),
        "the Vela worker directive survives: {page}"
    );
    assert_ne!(page, default_csp, "a page with a script is not the default");
    // No GIS on a page that does not load auth.js: the origin appears nowhere in its policy.
    assert!(
        !page.contains("accounts.google.com"),
        "only hub.html names the GIS origin: {page}"
    );

    // index.html has no inline block, so its header is exactly the default.
    let r = app.get(&base, "/index.html").await;
    assert_eq!(r.status(), 200, "index.html is served");
    assert_eq!(
        csp_of(&r),
        default_csp,
        "a script-free page gets the default"
    );

    // `/` resolves to index.html: the same default, not a missing header.
    let r = app.get(&base, "/").await;
    assert_eq!(r.status(), 200);
    assert_eq!(csp_of(&r), default_csp, "the root maps to index.html");

    // An API response carries the strict default, never a page policy.
    let r = app.get(&base, "/api/health").await;
    assert_eq!(r.status(), 200);
    let api = csp_of(&r);
    assert_eq!(api, default_csp, "/api/* carries the strict default");
    assert!(
        api.contains("default-src 'none'"),
        "the default denies by default: {api}"
    );

    // A 404 carries the default too: the fallback never answers without a policy.
    let r = app.get(&base, "/nope.html").await;
    assert_eq!(r.status(), 404);
    assert_eq!(csp_of(&r), default_csp, "a 404 carries the default");
    server.abort();
}

/// A local-mode app served the same way (no auth, so no harness state to expose).
struct LocalApp {
    app: axum::Router,
    client: reqwest::Client,
}

impl LocalApp {
    fn new(app: axum::Router) -> LocalApp {
        LocalApp {
            app,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    async fn serve(&self) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = self.app.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (format!("http://127.0.0.1:{port}"), server)
    }

    async fn get(&self, base: &str, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap()
    }
}
