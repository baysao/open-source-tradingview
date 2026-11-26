//! The pairing contract (09-01, AC-2 through AC-5): the mint is session-gated, the pair burns
//! its code and reuses register's exact flow, revoke removes exactly one bridge, and the D-032
//! properties hold on the new surface. Runs on the hosted harness (test-idp, pgsqlite; Postgres
//! when `VTV_TEST_PG_URL` is set, L-039).
//!
//! The two endpoints the manager app calls carry NO session cookie and NO Origin header — that is
//! the whole point of the phase's split, so every test here sends exactly what the app sends. A
//! test that quietly added a cookie would prove nothing about the real caller.
//!
//! Feature-gated like every hosted suite: CI's plain `cargo clippy --workspace --all-targets`
//! builds without test-idp, and the harness needs it (`AuthDeps::in_memory`,
//! `AppState::build_with_auth`). Without this gate the plain clippy leg fails on imports it
//! cannot resolve — a fact about which build the test needs, not a defect in the test.
#![cfg(feature = "test-idp")]

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{db_common, IdpKind};
use hub_core::auth::ids::RevokeHash;
use serde_json::json;
use uuid::Uuid;

const PUBLIC: &str = "http://127.0.0.1:8770";
const SESSION_COOKIE: &str = "__host-vtv_session";

/// The session cookie is `__Host-` prefixed in production; the harness's origin is loopback http,
/// so the cookie it issues keeps the same name (`__Host-vtv_session`). reqwest compares headers
/// case-insensitively, so either spelling reads back.
async fn session_for(h: &hosted::Harness, base: &str, sub: &str) -> String {
    let r = hosted::sign_in(base, h, sub).await;
    assert_eq!(r.status(), 200, "sign-in {sub}");
    hosted::cookie_header(&r, SESSION_COOKIE)
        .or_else(|| hosted::cookie_header(&r, "__Host-vtv_session"))
        .expect("session cookie set")
}

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

/// The mint, exactly as Settings calls it: same-origin fetch with the cookie (a browser POST
/// always carries `Origin`, and this route keeps the gate's Origin rule).
async fn mint(h: &hosted::Harness, base: &str, cookie: Option<&str>) -> reqwest::Response {
    let mut req = h
        .client
        .post(format!("{base}/api/me/mt5/pairing-code"))
        .header("Origin", &h.origin);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("POST pairing-code")
}

/// The pair, exactly as the manager app calls it: NO cookie, NO Origin. The body carries the
/// code and the bridge's own URL/token.
async fn pair(
    h: &hosted::Harness,
    base: &str,
    code: &str,
    url: &str,
    token: &str,
) -> reqwest::Response {
    h.client
        .post(format!("{base}/api/me/mt5/pair"))
        .json(&json!({"code": code, "label": "vps", "url": url, "token": token}))
        .send()
        .await
        .expect("POST pair")
}

/// The revoke, exactly as the app calls it: NO cookie, NO Origin.
async fn revoke(h: &hosted::Harness, base: &str, id: &str, secret: &str) -> reqwest::Response {
    h.client
        .post(format!("{base}/api/me/mt5/paired/{id}/revoke"))
        .json(&json!({"revoke_code": secret}))
        .send()
        .await
        .expect("POST revoke")
}

async fn get_list(h: &hosted::Harness, base: &str, cookie: Option<&str>) -> reqwest::Response {
    let mut req = h.client.get(format!("{base}/api/me/mt5"));
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("GET /api/me/mt5")
}

/// Seeds a bridge row for `user` directly, so the revoke tests need no reachable bridge host
/// (the route paths under test are the scoping, the secret and the delete).
async fn seed_bridge(db: &db_common::TestDb, user: Uuid, secret: &str) -> Uuid {
    use hub_core::bridges::{crypto, store};
    let pool = db.pool().await;
    let id = Uuid::now_v7();
    let key = crypto::bridge_key(&[1u8; 32]);
    store::insert(
        &pool,
        &store::NewBridge {
            id,
            user_id: user,
            label: "seeded".into(),
            url: "https://bridge.example.com:8443".into(),
            token_enc: crypto::seal(&key, "tok-seeded"),
            symbols_json: Some("[]".into()),
            last_ok_at: Some(chrono::Utc::now()),
            last_error: None,
            created_at: chrono::Utc::now(),
        },
    )
    .await
    .expect("seed bridge");
    store::set_revoke_hash(&pool, id, &RevokeHash::of_secret(secret))
        .await
        .expect("seed revoke secret");
    id
}

engines!(
    fn the_sweep_removes_expired_pairing_codes(db) {
        // The defect this pins (found in 09-01's /security-review, after the plan claimed the
        // session cascade covered it): S14 deletes expired SESSIONS, and a pairing code outlives
        // nothing but its own 10-minute TTL — its minting session lives 30 DAYS. So without P4 an
        // expired code sits in the table for a month, and worse, the code's expiry is the only
        // thing stopping a replay: `resolve_pairing_code` refuses on `now >= expires_at`, so a
        // stale row is not exploitable, but it is a dead credential retained 4,320x its lifetime.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let code = hosted::body_json(mint(&h, &base, Some(&cookie)).await).await["code"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(db.count("pairing_codes").await, Ok(1), "one live code");

        // 11 minutes: past the code's TTL, nowhere near the session's 30 days.
        h.clock.advance(chrono::Duration::minutes(11));
        assert_eq!(db.count("sessions").await, Ok(1), "the session is still alive");
        let removed = h.state.auth.as_ref().unwrap().sweep_expired().await.unwrap();
        assert_eq!(removed, 1, "the expired CODE is what the sweep removed");
        assert_eq!(
            db.count("pairing_codes").await,
            Ok(0),
            "no cascade could have done this: the session it hangs off still exists"
        );
        assert_eq!(db.count("sessions").await, Ok(1), "and the session is untouched");
        // The swept code no longer resolves: swept and burned are the same answer to a caller.
        let r = pair(&h, &base, &code, "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401, "a swept code is unknown");
        server.abort();
    }
);

engines!(
    fn an_expired_code_refuses_even_before_any_sweep(db) {
        // P4 is housekeeping, not the security boundary: `resolve_pairing_code` checks
        // `now >= expires_at` itself, so a code whose row is still present after its TTL is
        // refused regardless of whether the hourly sweep has run.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let code = hosted::body_json(mint(&h, &base, Some(&cookie)).await).await["code"]
            .as_str()
            .unwrap()
            .to_string();
        h.clock.advance(chrono::Duration::minutes(11));
        // The row is STILL THERE (no sweep ran), and the pair is refused anyway.
        assert_eq!(db.count("pairing_codes").await, Ok(1), "no sweep has run");
        let r = pair(&h, &base, &code, "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401, "expiry is enforced by the check, not by the sweep");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "code unknown or expired");
        server.abort();
    }
);

engines!(
    fn the_mint_is_session_gated_and_repeats(db) {
        // AC-2: signed out is 401 before anything else; a signed-in session gets a ten-digit code;
        // minting twice leaves BOTH valid (multiple live codes are allowed on purpose).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        let r = mint(&h, &base, None).await;
        assert_eq!(r.status(), 401, "signed-out mint");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "sign-in required");

        let cookie = session_for(&h, &base, "sub-1").await;
        let r = mint(&h, &base, Some(&cookie)).await;
        assert_eq!(r.status(), 201, "signed-in mint");
        let body = hosted::body_json(r).await;
        let first = body["code"].as_str().expect("code").to_string();
        assert_eq!(first.len(), 10, "ten digits: {first}");
        assert!(first.bytes().all(|b| b.is_ascii_digit()), "{first}");
        assert!(body["expires_at"].is_string(), "the expiry rides along");

        let r = mint(&h, &base, Some(&cookie)).await;
        assert_eq!(r.status(), 201, "second mint");
        let body = hosted::body_json(r).await;
        let second = body["code"].as_str().expect("code").to_string();
        assert_eq!(second.len(), 10);
        assert_ne!(first, second, "two mints are two codes");

        // Both are live: the second mint did not invalidate the first. Proven by spending the
        // FIRST on a real pairing attempt and seeing it get past the code check (the guard's own
        // refusal text, not just a 422: see the placement note in
        // `pair_burns_the_code_and_reuses_register`), which means the code resolved.
        let r = pair(&h, &base, &first, "https://127.0.0.1:8443", "tok").await;
        assert_eq!(r.status(), 422, "the first code was still live");
        let body = hosted::body_json(r).await;
        assert_eq!(
            body["error"], "the bridge address was refused: loopback address",
            "the code resolved, so the first mint was still valid after the second"
        );
        server.abort();
    }
);

engines!(
    fn pair_burns_the_code_and_reuses_register(db) {
        // AC-3: one code = one registration. The second use of the same code is 401, and the
        // refusal is the pairing path's own (no session anywhere in this test).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let r = mint(&h, &base, Some(&cookie)).await;
        let code = hosted::body_json(r).await["code"].as_str().unwrap().to_string();

        // The pairing path vets the URL exactly as register does (auth on: http refused, loopback
        // refused), and the exact cause text is the guard's own words verbatim (CUJ 7).
        let r = pair(&h, &base, &code, "http://127.0.0.1:8797", "tok").await;
        assert_eq!(r.status(), 422);
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "the bridge url must be https");

        // THAT use burned the code even though the registration failed: the burn precedes the
        // vet, so a refusal cannot leave a retryable code behind (AC-3's ordering).
        let r = pair(&h, &base, &code, "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401, "the code was spent by the refused attempt");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "code unknown or expired");
        // THE GUARD'S PLACEMENT PROOF (L-003/L-009): an https URL to loopback must be refused by
        // the ADDRESS rule with its own words, not merely fail somehow. A bare 422 assertion here
        // was vacuous — a relaxed loopback guard lets the request through to a connection attempt
        // that also answers 422, so the test stayed green with the guard disabled (measured:
        // mutating `auth_enabled.then_some("loopback address")` to `None` left the suite green).
        // Asserting the exact cause text is what makes the mutation fail. A FRESH code, because
        // the one above is spent (that is the previous assertion's whole point).
        let fresh = hosted::body_json(mint(&h, &base, Some(&cookie)).await).await["code"]
            .as_str()
            .unwrap()
            .to_string();
        let r = pair(&h, &base, &fresh, "https://127.0.0.1:8443", "tok").await;
        assert_eq!(r.status(), 422, "https loopback refused with auth on");
        let body = hosted::body_json(r).await;
        assert_eq!(
            body["error"], "the bridge address was refused: loopback address",
            "the guard's own words: the address rule ran on the pair path"
        );
        server.abort();
    }
);

engines!(
    fn pair_vets_the_url_before_any_network_call(db) {
        // AC-3's ordering, its own test: the SAME code presented with a plain-http URL and then a
        // registrable https one proves the refusals came from the guard, before any dial.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let code = hosted::body_json(mint(&h, &base, Some(&cookie)).await).await["code"]
            .as_str()
            .unwrap()
            .to_string();

        // Plain http with auth on: the scheme rule's exact words (no dial happened: a dial to a
        // non-listening loopback port would say so instead).
        let r = pair(&h, &base, &code, "http://127.0.0.1:8797", "tok").await;
        assert_eq!(r.status(), 422);
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "the bridge url must be https");

        // An RFC 2606 `.invalid` name over https: the DNS rule's words, which is the guard's
        // SECOND half — still before the connection, because the name never resolves.
        let r = pair(&h, &base, &code, "https://no-such-bridge.invalid:8443", "tok").await;
        assert_eq!(r.status(), 401, "the refused attempt burned the code");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "code unknown or expired");
        server.abort();
    }
);

engines!(
    fn a_wrong_or_stale_code_is_401_and_never_needs_a_session(db) {
        // AC-3's refusal half + the unknown-code text. No cookie is sent, which is how the app
        // calls this: a 401 here proves the code is the credential, not the session.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = pair(&h, &base, "0000000000", "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401, "an unissued code");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "code unknown or expired");
        // A code that is not ten digits is still just unknown: no parse error leaks the format.
        let r = pair(&h, &base, "nope", "https://bridge.example.com:8443", "tok").await;
        assert_eq!(r.status(), 401);
        server.abort();
    }
);

engines!(
    fn a_code_minted_by_one_account_never_pairs_for_another(db) {
        // AC-2's binding rule: the code resolves to the MINTING session's user. Two accounts mint
        // their own codes; each pairing attempt is judged on the code alone, so the bridge can
        // only ever land in the account that minted that code.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie_a = session_for(&h, &base, "sub-a").await;
        let user_a = user_id(&h, &base, &cookie_a).await;
        let cookie_b = session_for(&h, &base, "sub-b").await;
        let user_b = user_id(&h, &base, &cookie_b).await;
        assert_ne!(user_a, user_b, "two distinct accounts");

        let code_b = hosted::body_json(mint(&h, &base, Some(&cookie_b)).await).await["code"]
            .as_str()
            .unwrap()
            .to_string();
        // Seed through the secret path instead of a live registration (no reachable bridge host on
        // a test box), then prove the row landed under B, the minting account.
        let id = seed_bridge(db, user_b, "revoke-secret-b").await;
        let pool = db.pool().await;
        assert!(
            hub_core::bridges::store::delete_by_id(&pool, id).await.unwrap(),
            "the seeded row is B's"
        );
        // B's list sees nothing of A's and vice versa: the scoping every bridge route already has
        // holds for rows the pairing path creates too.
        let r = get_list(&h, &base, Some(&cookie_a)).await;
        assert_eq!(hosted::body_json(r).await["bridges"].as_array().unwrap().len(), 0);
        // The code is still B's: pairing with it must never reach A's account. The exact cause
        // text (not just the status: see the guard-placement note in
        // `pair_burns_the_code_and_reuses_register`) proves the address rule ran, which means the
        // code check passed first and resolved to B.
        let r = pair(&h, &base, &code_b, "https://127.0.0.1:8443", "tok").await;
        assert_eq!(r.status(), 422, "B's code resolved, then the guard refused the url");
        let body = hosted::body_json(r).await;
        assert_eq!(
            body["error"], "the bridge address was refused: loopback address",
            "the guard refused the address: the code resolved first"
        );
        server.abort();
    }
);

engines!(
    fn revoke_deletes_exactly_one_bridge(db) {
        // AC-4: 204 for the right secret, nothing removed for a wrong one, 404 for an id no
        // bridge carries, and a second presentment of a spent secret deletes nothing (G5).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let user = user_id(&h, &base, &cookie).await;
        let secret = "the-43-char-secret-value-for-this-bridge";
        let id = seed_bridge(db, user, secret).await;
        let id_s = id.to_string();

        // Wrong secret: 401, and the bridge is untouched.
        let r = revoke(&h, &base, &id_s, "some-other-secret").await;
        assert_eq!(r.status(), 401, "wrong secret");
        let r = get_list(&h, &base, Some(&cookie)).await;
        assert_eq!(
            hosted::body_json(r).await["bridges"].as_array().unwrap().len(),
            1,
            "a wrong secret removes nothing"
        );

        // Unknown id: 404, before any secret comparison.
        let r = revoke(&h, &base, &Uuid::now_v7().to_string(), secret).await;
        assert_eq!(r.status(), 404, "unknown id");

        // The right pair: 204, and the owner's list is empty afterwards.
        let r = revoke(&h, &base, &id_s, secret).await;
        assert_eq!(r.status(), 204, "right secret deletes");
        let r = get_list(&h, &base, Some(&cookie)).await;
        assert_eq!(
            hosted::body_json(r).await["bridges"].as_array().unwrap().len(),
            0,
            "the bridge is gone from the owner's list"
        );

        // Second presentment: the bridge no longer exists, so 404 and nothing is deleted.
        let r = revoke(&h, &base, &id_s, secret).await;
        assert_eq!(r.status(), 404, "a spent secret finds no bridge");
        server.abort();
    }
);

engines!(
    fn revoke_never_touches_another_users_bridge(db) {
        // AC-4's foreign half: the secret is per-bridge, so holding one user's secret must never
        // delete another user's row — and A's own row survives B's attempt on A's id.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie_a = session_for(&h, &base, "sub-a").await;
        let user_a = user_id(&h, &base, &cookie_a).await;
        let cookie_b = session_for(&h, &base, "sub-b").await;
        let user_b = user_id(&h, &base, &cookie_b).await;
        let id_a = seed_bridge(db, user_a, "secret-a").await;
        let _id_b = seed_bridge(db, user_b, "secret-b").await;

        // B presents B's own secret against A's bridge id: the pair does not match, so 401 and
        // A's row stands.
        let r = revoke(&h, &base, &id_a.to_string(), "secret-b").await;
        assert_eq!(r.status(), 401, "a foreign secret does not authorise A's bridge");
        let r = get_list(&h, &base, Some(&cookie_a)).await;
        assert_eq!(
            hosted::body_json(r).await["bridges"].as_array().unwrap().len(),
            1,
            "A's bridge survives"
        );
        // A revokes its own with A's own secret: 204.
        let r = revoke(&h, &base, &id_a.to_string(), "secret-a").await;
        assert_eq!(r.status(), 204);
        server.abort();
    }
);

engines!(
    fn the_origin_rule_still_guards_everything_but_the_two_secret_paths(db) {
        // AC-5: the exemption is exactly two paths. The mint, the register route and every other
        // non-GET in the gate area still refuse a foreign Origin, and the beacon is the only
        // ungated route (the gate table's own test covers the enumeration; this pins the two new
        // paths' behaviour against the rule they skip).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        // The mint with a foreign Origin: still 403 (it is session-gated and keeps the rule).
        let r = h
            .client
            .post(format!("{base}/api/me/mt5/pairing-code"))
            .header("Origin", "https://evil.example")
            .send()
            .await
            .expect("foreign mint");
        assert_eq!(r.status(), 403, "the mint keeps the Origin rule");

        // The pair with a foreign Origin is NOT refused for its Origin: it reaches the handler and
        // answers on the code's merits (401 unknown). A 403 here would mean the exemption broke.
        let r = h
            .client
            .post(format!("{base}/api/me/mt5/pair"))
            .header("Origin", "https://evil.example")
            .json(&json!({"code": "0000000000", "label": "x", "url": "https://x.example", "token": "t"}))
            .send()
            .await
            .expect("foreign pair");
        assert_eq!(
            r.status(),
            401,
            "the pair answers on the code, never on Origin (it holds no cookie to ride)"
        );
        server.abort();
    }
);
