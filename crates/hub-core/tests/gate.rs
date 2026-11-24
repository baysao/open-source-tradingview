//! The gate table (03-02 AC-5): every hosted non-GET route from `route_specs` is sent with no
//! Origin, with a foreign Origin and with PUBLIC_ORIGIN; GET `/auth/me` needs no Origin; every
//! hosted route lies in a gate area; text/plain is 415 and 300,001 bytes are 413. Plus AC-7's
//! mode table: local answers `{"auth": false}` and 404s the hosted routes; hosted 404s
//! `/api/logs*`.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{body_json, db_common, get, post_raw, IdpKind};
use serde_json::json;

const PUBLIC: &str = "http://127.0.0.1:8770";

engines!(
    fn every_hosted_route_is_in_a_gate_area(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let specs = hub_core::routes::table::route_specs(&h.state);
        assert!(!specs.is_empty(), "the hosted table is mounted");
        for s in &specs {
            let in_area = s.path.starts_with("/auth/")
                || s.path.starts_with("/admin/")
                || s.path == "/api/me"
                || s.path.starts_with("/api/me/");
            assert!(in_area, "{} {} lies outside every gate area", s.method, s.path);
        }
        // The area decision itself is unit-tested in `src/routes/gate.rs`; here the mounted
        // table is the contract: /api/metrics, if someone added it, would fail the loop above.
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
