//! The bridge route contract (07-03): registration refusals carry the exact cause (AC-2
//! through the route, auth-on: loopback refused, a DNS miss refused), the cross-user rule
//! (G5: a foreign id is 404 on test and delete, nothing of theirs is touched), the token
//! never appears in any response, signed-out is 401 on all four, and a failed re-test keeps
//! the last working state. Runs on the hosted harness (test-idp, pgsqlite; Postgres when
//! `VTV_TEST_PG_URL` is set, L-039).

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{db_common, IdpKind};
use hub_core::bridges::{crypto, store};
use serde_json::json;
use uuid::Uuid;

const PUBLIC: &str = "http://127.0.0.1:8770";
const SESSION_COOKIE: &str = "__Host-vtv_session";

/// Sign in as `sub` and return the Cookie header value.
async fn session_for(h: &hosted::Harness, base: &str, sub: &str) -> String {
    let r = hosted::sign_in(base, h, sub).await;
    assert_eq!(r.status(), 200, "sign-in {sub}");
    hosted::cookie_header(&r, SESSION_COOKIE).expect("session cookie set")
}

/// The signed-in user's id, from `/auth/me` (the id the route layer scopes by).
async fn user_id(h: &hosted::Harness, base: &str, cookie: &str) -> Uuid {
    let r = h
        .client
        .get(format!("{base}/auth/me"))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("GET /auth/me");
    let body = hosted::body_json(r).await;
    body["user"]["id"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .expect("the id rides in /auth/me")
}

async fn register(
    h: &hosted::Harness,
    base: &str,
    cookie: Option<&str>,
    url: &str,
    token: &str,
) -> reqwest::Response {
    let mut req = h
        .client
        .post(format!("{base}/api/me/mt5"))
        .header("Origin", &h.origin)
        .json(&json!({"label": "home", "url": url, "token": token}));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("POST /api/me/mt5")
}

async fn get_list(h: &hosted::Harness, base: &str, cookie: Option<&str>) -> reqwest::Response {
    let mut req = h.client.get(format!("{base}/api/me/mt5"));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("GET /api/me/mt5")
}

async fn test_bridge(h: &hosted::Harness, base: &str, cookie: &str, id: &str) -> reqwest::Response {
    h.client
        .post(format!("{base}/api/me/mt5/{id}/test"))
        .header("Origin", &h.origin)
        .header("Cookie", cookie)
        .send()
        .await
        .expect("POST test")
}

async fn delete_bridge(
    h: &hosted::Harness,
    base: &str,
    cookie: Option<&str>,
    id: &str,
) -> reqwest::Response {
    let mut req = h
        .client
        .delete(format!("{base}/api/me/mt5/{id}"))
        .header("Origin", &h.origin);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("DELETE")
}

engines!(
    fn register_refuses_with_the_exact_cause(db) {
        // AC-2 through the route, auth on: a loopback URL is refused before any connection,
        // and a name that resolves to nothing is refused too. Both answers carry the guard's
        // own words verbatim (CUJ 7).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        // Plain http with auth on: the scheme rule refuses before any dial.
        let r = register(&h, &base, Some(&cookie), "http://127.0.0.1:8797", "tok").await;
        assert_eq!(r.status(), 422, "plain http refused with auth on");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "the bridge url must be https");
        // https to loopback: the address rule refuses (the guard's own words, verbatim).
        let r = register(&h, &base, Some(&cookie), "https://127.0.0.1:8443", "tok").await;
        assert_eq!(r.status(), 422, "loopback refused with auth on");
        let body = hosted::body_json(r).await;
        assert_eq!(
            body["error"], "the bridge address was refused: loopback address",
            "the exact refusal text, verbatim"
        );
        // A reserved name that cannot resolve (RFC 2606 .invalid): the DNS-miss cause.
        let r = register(
            &h,
            &base,
            Some(&cookie),
            "https://no-such-bridge.invalid:8443",
            "tok",
        )
        .await;
        assert_eq!(r.status(), 422, "dns miss refused");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "the bridge host did not resolve");
        server.abort();
    }
);

engines!(
    fn cross_user_is_404_and_the_token_never_leaves(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie_a = session_for(&h, &base, "sub-1").await;
        let cookie_b = session_for(&h, &base, "sub-2").await;
        let user_a = user_id(&h, &base, &cookie_a).await;
        // Seed the owner's row directly (a registrable public https bridge does not exist on
        // a test box; the route paths under test are the scoping and the shape). The key is
        // KEY32's bytes, which the harness's server derived its bridge key from.
        let pool = db.pool().await;
        let id = Uuid::now_v7();
        let key = crypto::bridge_key(&[1u8; 32]); // KEY32's bytes: 32 bytes of 0x01
        let token = "tok-bridge-42";
        store::insert(
            &pool,
            &store::NewBridge {
                id,
                user_id: user_a,
                label: "home".into(),
                url: "https://bridge.example.com:8443".into(),
                token_enc: crypto::seal(&key, token),
                symbols_json: Some(r#"[{"name":"EURUSD","desc":"","path":"Other","digits":5}]"#.into()),
                last_ok_at: Some(chrono::Utc::now()),
                last_error: None,
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .expect("seed row");
        // The owner sees it; the token appears in no field of the response.
        let r = get_list(&h, &base, Some(&cookie_a)).await;
        assert_eq!(r.status(), 200, "owner list");
        let text = r.text().await.expect("owner body");
        assert!(text.contains("home"), "{text}");
        assert!(!text.contains(token), "the token never leaves the server: {text}");
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        let bridges = body["bridges"].as_array().expect("bridges array");
        assert_eq!(bridges.len(), 1);
        assert_eq!(bridges[0]["symbol_count"], 1, "symbol_count from the cache");
        assert_eq!(bridges[0]["connected"], true);
        // The other user sees nothing, and cannot test or delete what is not theirs (G5).
        let r = get_list(&h, &base, Some(&cookie_b)).await;
        let body = hosted::body_json(r).await;
        assert_eq!(body["bridges"].as_array().unwrap().len(), 0, "B owns nothing");
        let id_s = id.to_string();
        let r = test_bridge(&h, &base, &cookie_b, &id_s).await;
        assert_eq!(r.status(), 404, "foreign test is 404");
        let r = delete_bridge(&h, &base, Some(&cookie_b), &id_s).await;
        assert_eq!(r.status(), 404, "foreign delete is 404");
        // The owner can still test and delete; a second delete is 404 again.
        let r = test_bridge(&h, &base, &cookie_a, &id_s).await;
        // The seeded url re-vets (auth on) as a public https host and fails DNS: 502 with the
        // exact cause, and the failure keeps the last working state.
        assert_eq!(r.status(), 502, "the re-test fails on the fake host");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "the bridge host did not resolve");
        let r = get_list(&h, &base, Some(&cookie_a)).await;
        let body = hosted::body_json(r).await;
        assert_eq!(body["bridges"][0]["last_error"], "the bridge host did not resolve");
        assert_eq!(
            body["bridges"][0]["connected"], true,
            "a failed re-test keeps last_ok_at (the cache is from the last successful test)"
        );
        let r = delete_bridge(&h, &base, Some(&cookie_a), &id_s).await;
        assert_eq!(r.status(), 204, "owner delete");
        let r = delete_bridge(&h, &base, Some(&cookie_a), &id_s).await;
        assert_eq!(r.status(), 404, "already deleted");
        server.abort();
    }
);

engines!(
    fn signed_out_is_401_on_all_four(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = register(&h, &base, None, "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401, "signed-out POST");
        let r = get_list(&h, &base, None).await;
        assert_eq!(r.status(), 401, "signed-out GET");
        let r = test_bridge(&h, &base, "", &Uuid::now_v7().to_string()).await;
        assert_eq!(r.status(), 401, "signed-out test");
        let r = delete_bridge(&h, &base, None, &Uuid::now_v7().to_string()).await;
        assert_eq!(r.status(), 401, "signed-out DELETE");
        // The bridge bars path is session-scoped too (m1's mutation targets this check).
        let r = h
            .client
            .get(format!(
                "{base}/api/bars?source=mt5&bridge={}&symbol=X&tf=H1",
                Uuid::now_v7()
            ))
            .send()
            .await
            .expect("signed-out bridge bars");
        assert_eq!(r.status(), 401, "signed-out bridge bars is 401");
        server.abort();
    }
);
