//! The auth flows through `app(state)` on both engines (03-02: AC-2, AC-3, AC-4, AC-6, the
//! HTTP IdP path, the log-ring secret scan). No Google call: the in-memory TestIdp and, for
//! the HTTP path, the served `/__test-idp/*` router.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{
    body_json, cookie_header, cookie_value, db_common, get, post_json, set_cookies, sign_in,
    IdpKind, CLIENT_ID,
};
use hub_core::auth::test_idp::Claims;
use serde_json::json;

const SESSION: &str = "__Host-vtv_session";
const OAUTH: &str = "__Host-vtv_oauth";

// --- AC-2: One Tap creates one user, the second reuses it ---------------------------

engines!(
    fn first_sign_in_creates_one_user_second_reuses(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        let r = sign_in(&base, &h, "g-1").await;
        assert_eq!(r.status(), 200, "{}", r.text().await.unwrap_or_default());
        let body = body_json(r).await;
        assert_eq!(body["user"]["email"], "g-1@example.test");
        assert_eq!(body["admin"], false);
        assert_eq!(db.count("users").await, Ok(1));
        assert_eq!(db.count("identities").await, Ok(1));
        // last_login_at is a real time: the second sign-in must move it.
        let first_login: (chrono::DateTime<chrono::Utc>,) =
            sqlx::query_as("SELECT last_login_at FROM users")
                .fetch_one(&db.pool().await)
                .await
                .unwrap();

        h.clock.advance(chrono::Duration::hours(1));
        let r2 = sign_in(&base, &h, "g-1").await;
        assert_eq!(r2.status(), 200);
        assert_eq!(db.count("users").await, Ok(1), "the second sign-in reuses the user");
        assert_eq!(db.count("identities").await, Ok(1));
        let second_login: (chrono::DateTime<chrono::Utc>,) =
            sqlx::query_as("SELECT last_login_at FROM users")
                .fetch_one(&db.pool().await)
                .await
                .unwrap();
        assert!(second_login.0 > first_login.0, "last_login_at updated");
        server.abort();
    }
);

engines!(
    fn session_cookie_shape_and_hash_storage(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = sign_in(&base, &h, "g-1").await;
        let cookies = set_cookies(&r);
        let session = cookies
            .iter()
            .find(|c| c.starts_with(SESSION))
            .expect("session cookie set");
        assert!(session.contains("HttpOnly"), "{session}");
        assert!(session.contains("Secure"), "{session}");
        assert!(session.contains("SameSite=Lax"), "{session}");
        assert!(session.contains("Path=/"), "{session}");
        assert!(session.contains("Max-Age=2592000"), "{session}");
        let value = cookie_value(&r, SESSION).expect("value");
        assert_eq!(value.len(), 43, "32 bytes base64url unpadded: {value}");
        // The row keeps the hash, never the raw value.
        let (id_hash,): (Vec<u8>,) =
            sqlx::query_as("SELECT id_hash FROM sessions").fetch_one(&db.pool().await).await.unwrap();
        let digest = sha256_hex(&value);
        assert_eq!(hex(&id_hash), digest, "sessions.id_hash is SHA-256 of the cookie value");
        assert_ne!(&id_hash, &value.as_bytes(), "the raw value is never stored");
        server.abort();
    }
);

engines!(
    fn forged_credentials_are_401_and_create_nothing(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        // Wrong audience.
        let wrong_aud = h.idp.id_token(&Claims::for_sub("g-1").aud("someone-else"), None, 0);
        // Wrong issuer.
        let wrong_iss = h
            .idp
            .id_token(&Claims::for_sub("g-1").aud(CLIENT_ID).iss("https://evil.example"), None, 0);
        // exp == now (refused: exp must be strictly in the future), by the harness's clock.
        let now = h.clock.now().timestamp();
        let expired = h.idp.id_token(
            &Claims::for_sub("g-1").aud(CLIENT_ID).exp(Some(Some(now))),
            None,
            0,
        );
        // exp in the past.
        let past = h.idp.id_token(
            &Claims::for_sub("g-1").aud(CLIENT_ID).exp(Some(Some(now - 60))),
            None,
            0,
        );
        // Bad signature: flip one character in the middle of the signature part (flipping the
        // last base64url character can be a no-op for 2048-bit signatures' unused tail bits).
        let full = h.idp.id_token(&Claims::for_sub("g-1").aud(CLIENT_ID), None, 0);
        let (head, sig) = full.rsplit_once('.').expect("three parts");
        let mut sig_chars: Vec<char> = sig.chars().collect();
        let mid = sig_chars.len() / 2;
        sig_chars[mid] = if sig_chars[mid] == 'A' { 'B' } else { 'A' };
        let tampered = format!("{head}.{}", sig_chars.into_iter().collect::<String>());
        // alg none.
        let none = format!(
            "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.{}.x",
            b64url(br#"{"iss":"https://accounts.google.com","sub":"g-1","aud":"test-client-id","email":"g-1@example.test","email_verified":true,"exp":9999999999}"#)
        );
        // alg HS256.
        let hs256 = format!(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.{}.c2ln",
            b64url(br#"{"iss":"https://accounts.google.com","sub":"g-1","aud":"test-client-id","email":"g-1@example.test","email_verified":true,"exp":9999999999}"#)
        );
        // A token signed by a key the JWKS does not name (rotation): refused before any
        // claim check (the missing sub/email edges are pinned in the rules unit tests,
        // where a minted token without the claim exists).
        let unknown_kid = h.idp.id_token_unknown_kid(
            &Claims::for_sub("g-1").aud(CLIENT_ID),
            None,
        );

        for (name, credential) in [
            ("wrong aud", wrong_aud),
            ("wrong iss", wrong_iss),
            ("exp == now", expired),
            ("exp past", past),
            ("bad signature", tampered),
            ("alg none", none),
            ("alg hs256", hs256),
            ("unknown kid", unknown_kid),
            ("garbage", "not-a-jwt".to_string()),
        ] {
            let r = post_json(
                &base,
                &h,
                "/auth/google/onetap",
                json!({"credential": credential}),
            )
            .await;
            assert_eq!(r.status(), 401, "{name}");
            assert_eq!(body_json(r).await["error"], "sign-in failed", "{name}");
        }
        assert_eq!(db.count("users").await, Ok(0), "no row for any refusal");
        assert_eq!(db.count("identities").await, Ok(0));
        assert_eq!(db.count("sessions").await, Ok(0));
        server.abort();
    }
);

engines!(
    fn two_concurrent_first_sign_ins_make_one_user(db) {
        let h = std::sync::Arc::new(
            hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await,
        );
        let (base, server) = h.serve().await;
        let token = hosted::one_tap_token(&h, "g-race");

        let mut tasks = Vec::new();
        for _ in 0..2 {
            let client = h.client.clone();
            let origin = h.origin.clone();
            let url = format!("{base}/auth/google/onetap");
            let body = json!({"credential": token.clone()});
            tasks.push(tokio::spawn(async move {
                client
                    .post(&url)
                    .header("Origin", origin)
                    .json(&body)
                    .send()
                    .await
                    .expect("POST")
            }));
        }
        let mut oks = 0;
        for t in tasks {
            let r = t.await.unwrap();
            assert!(r.status() == 200 || r.status() == 401, "racer got {}", r.status());
            if r.status() == 200 {
                oks += 1;
            }
        }
        assert_eq!(oks, 2, "both concurrent first sign-ins answer 200 (retry hits)");
        assert_eq!(db.count("users").await, Ok(1), "exactly one user (the loser rolled back)");
        assert_eq!(db.count("identities").await, Ok(1));
        assert_eq!(db.count("sessions").await, Ok(2), "each winner has a session");
        server.abort();
    }
);

// --- AC-3: the web button flow and the oauth cookie ---------------------------------

engines!(
    fn button_flow_sets_pkce_and_lands_signed_in(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let _log = hosted::feed_ring(&h);
        let (base, server) = h.serve().await;

        let r = get(&base, &h, "/auth/google/start?client=web").await;
        assert_eq!(r.status(), 302);
        let loc = r.headers().get("Location").unwrap().to_str().unwrap().to_string();
        assert!(loc.starts_with("/__test-idp/authorize?"), "{loc}");
        assert!(loc.contains("code_challenge_method=S256"), "{loc}");
        assert!(loc.contains("code_challenge="), "{loc}");
        assert!(loc.contains("nonce="), "{loc}");
        let cookies = set_cookies(&r);
        let oauth = cookies.iter().find(|c| c.starts_with(OAUTH)).expect("oauth cookie");
        assert!(oauth.contains("Max-Age=600"), "{oauth}");
        assert!(oauth.contains("HttpOnly") && oauth.contains("Secure") && oauth.contains("SameSite=Lax"));

        // Follow the flow: authorize (grants a code), then callback with the ticket cookie.
        let auth = h.client.get(format!("{base}{loc}")).send().await.unwrap();
        assert_eq!(auth.status(), 302, "authorize redirects back with the code");
        let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
        let code = query(&back, "code").expect("granted code");
        let state = query(&back, "state").expect("state");

        let cb = h
            .client
            .get(format!("{base}/auth/google/callback"))
            .query(&[("state", state.as_str()), ("code", code.as_str())])
            .header("Cookie", cookie_header(&r, OAUTH).expect("ticket"))
            .send()
            .await
            .unwrap();
        assert_eq!(cb.status(), 302, "{}", cb.text().await.unwrap_or_default());
        let loc = cb.headers().get("Location").unwrap().to_str().unwrap();
        assert_eq!(loc, "/hub.html", "the web landing");
        assert!(cookie_value(&cb, SESSION).is_some(), "session cookie set");
        // The oauth cookie is cleared (Max-Age=0) and the session row exists.
        let cleared = set_cookies(&cb).iter().any(|c| c.starts_with(&format!("{OAUTH}=;")) && c.contains("Max-Age=0"));
        assert!(cleared, "oauth cookie cleared: {:?}", set_cookies(&cb));
        assert_eq!(db.count("users").await, Ok(1));
        server.abort();
    }
);

engines!(
    fn failed_callbacks_land_on_signin_failed_and_clear_the_ticket(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        async fn start_flow(h: &hosted::Harness, base: &str) -> (reqwest::Response, String, String) {
            let r = get(base, h, "/auth/google/start?client=web").await;
            let auth = h.client.get(format!("{base}{}", r.headers().get("Location").unwrap().to_str().unwrap())).send().await.unwrap();
            let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
            let code = query(&back, "code").unwrap();
            let state = query(&back, "state").unwrap();
            (r, code, state)
        }
        async fn expect_failed(
            h: &hosted::Harness,
            base: &str,
            ticket: Option<String>,
            state: &str,
            code: &str,
            what: &str,
        ) {
            let mut req = h
                .client
                .get(format!("{base}/auth/google/callback"))
                .query(&[("state", state), ("code", code)]);
            if let Some(t) = ticket {
                req = req.header("Cookie", format!("{OAUTH}={t}"));
            }
            let cb = req.send().await.unwrap();
            assert_eq!(cb.status(), 302, "{what}");
            assert_eq!(
                cb.headers().get("Location").unwrap().to_str().unwrap(),
                "/hub.html?signin=failed",
                "{what}"
            );
            assert!(
                set_cookies(&cb).iter().any(|c| c.starts_with(&format!("{OAUTH}=;")) && c.contains("Max-Age=0")),
                "{what}: oauth cookie cleared"
            );
        }

        // A valid start, then a tampered ticket (flip a payload byte: MAC fails).
        let (r, code, state) = start_flow(&h, &base).await;
        let ticket = cookie_value(&r, OAUTH).unwrap();
        let mut tampered = ticket.clone();
        let mid = tampered.len() / 2;
        tampered.replace_range(mid..mid + 1, if &tampered[mid..mid + 1] == "A" { "B" } else { "A" });
        expect_failed(&h, &base, Some(tampered), &state, &code, "tampered ticket").await;
        assert_eq!(db.count("users").await, Ok(0));

        // Expired ticket: seal one 601 s ago by walking the clock past Max-Age and reusing
        // the original ticket (its exp is inside).
        let (r2, code2, state2) = start_flow(&h, &base).await;
        h.clock.advance(chrono::Duration::seconds(601));
        let t2 = cookie_value(&r2, OAUTH).unwrap();
        expect_failed(&h, &base, Some(t2), &state2, &code2, "expired ticket").await;
        h.clock.advance(chrono::Duration::seconds(-601));

        // A token whose nonce does not match the ticket: the memory Idp mints with the
        // ticket's nonce, so drive the HTTP-free path by tampering the ticket's nonce:
        // start a flow, then complete it with a ticket whose nonce claim is unusable.
        // (The Idp mints from the ticket, so this case is pinned by the rules unit test
        // `refusal_reasons_order_the_checks` and by the HTTP path in http_idp.)

        // Wrong state, absent cookie, error=access_denied.
        let (r3, code3, _state3) = start_flow(&h, &base).await;
        let t3 = cookie_value(&r3, OAUTH).unwrap();
        expect_failed(&h, &base, Some(t3), "wrong-state", &code3, "wrong state").await;

        expect_failed(&h, &base, None, "anything", "anything", "absent cookie").await;

        let cb = h
            .client
            .get(format!("{base}/auth/google/callback"))
            .query(&[("state", "s"), ("code", "c"), ("error", "access_denied")])
            .header("Cookie", format!("{OAUTH}={}", ticket))
            .send()
            .await
            .unwrap();
        assert_eq!(cb.status(), 302);
        assert_eq!(cb.headers().get("Location").unwrap().to_str().unwrap(), "/hub.html?signin=failed");
        assert_eq!(db.count("users").await, Ok(0), "no sign-in from any failure");
        assert_eq!(db.count("sessions").await, Ok(0));
        server.abort();
    }
);

engines!(
    fn http_idp_path_round_trips_the_verifier(db) {
        // AC-3's PKCE proof over real HTTP: google.rs posts the verifier; the IdP checks
        // S256(verifier) == challenge and answers the id token.
        let (h, base, server) = hosted::Harness::over_http(db, &[]).await;

        let r = h
            .client
            .get(format!("{base}/auth/google/start?client=web"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 302);
        let loc = r.headers().get("Location").unwrap().to_str().unwrap().to_string();
        assert!(loc.starts_with("http"), "a real URL: {loc}");
        let jwks_before = h.idp.jwks_fetches.load(std::sync::atomic::Ordering::SeqCst);

        let auth = h.client.get(&loc).send().await.unwrap();
        assert_eq!(auth.status(), 302);
        let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
        let code = query(&back, "code").expect("code");
        let state = query(&back, "state").expect("state");

        let cb = h
            .client
            .get(format!("{base}/auth/google/callback"))
            .query(&[("state", state.as_str()), ("code", code.as_str())])
            .header("Cookie", cookie_header(&r, OAUTH).expect("ticket"))
            .send()
            .await
            .unwrap();
        assert_eq!(cb.status(), 302, "{}", cb.text().await.unwrap_or_default());
        assert_eq!(cb.headers().get("Location").unwrap().to_str().unwrap(), "/hub.html");
        assert!(cookie_value(&cb, SESSION).is_some());
        let jwks_after = h.idp.jwks_fetches.load(std::sync::atomic::Ordering::SeqCst);
        assert!(jwks_after > jwks_before, "the HTTP Idp fetched the JWKS");
        let verifier = h.idp.last_verifier.lock().unwrap_or_else(|p| p.into_inner()).clone();
        assert!(verifier.is_some(), "the Idp saw the code_verifier (PKCE sent)");
        assert_eq!(db.count("users").await, Ok(1));
        server.abort();
    }
);

// --- AC-4: desktop codes are one-time and bound to the key (D-015) ------------------

const GOOD_KEY: &str = "abc-def_ghi-jkl_mno-pqr_stu-vwx_yza-bcd_efg";

fn other_key() -> String {
    "zzz-def_ghi-jkl_mno-pqr_stu-vwx_yza-bcd_efg".to_string()
}

engines!(
    fn desktop_code_is_one_time_and_bound(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        // A bad dk is 400 bad request.
        let r = get(&base, &h, "/auth/google/start?client=desktop&dk=short").await;
        assert_eq!(r.status(), 400);
        assert_eq!(body_json(r).await["error"], "bad request");

        // The desktop flow: start, authorize, callback lands on the deep link.
        let r = get(&base, &h, &format!("/auth/google/start?client=desktop&dk={GOOD_KEY}")).await;
        assert_eq!(r.status(), 302);
        let auth = h.client.get(format!("{base}{}", r.headers().get("Location").unwrap().to_str().unwrap())).send().await.unwrap();
        let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
        let code = query(&back, "code").unwrap();
        let state = query(&back, "state").unwrap();
        let cb = h
            .client
            .get(format!("{base}/auth/google/callback"))
            .query(&[("state", state.as_str()), ("code", code.as_str())])
            .header("Cookie", cookie_header(&r, OAUTH).expect("ticket"))
            .send()
            .await
            .unwrap();
        assert_eq!(cb.status(), 302);
        let deep = cb.headers().get("Location").unwrap().to_str().unwrap().to_string();
        assert!(deep.starts_with("vtradingview://auth?code="), "{deep}");
        assert!(
            !set_cookies(&cb).iter().any(|c| c.starts_with(SESSION) && !c.contains("Max-Age=0")),
            "no usable session cookie: the placeholder is not handed out"
        );
        let exchange_code = deep.rsplit('=').next().unwrap().to_string();

        // The placeholder session expires in 60 s and is unusable.
        assert_eq!(db.count("sessions").await, Ok(1));
        h.clock.advance(chrono::Duration::seconds(61));
        let expired = post_json(
            &base,
            &h,
            "/auth/desktop/exchange",
            json!({"code": exchange_code, "desktop_key": GOOD_KEY}),
        )
        .await;
        assert_eq!(expired.status(), 401, "exchange at +61 s is 401");
        server.abort();
    }
);

engines!(
    fn desktop_exchange_burns_the_code_on_every_attempt(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;

        async fn desktop_flow(h: &hosted::Harness, base: &str, dk: &str) -> String {
            let r = get(base, h, &format!("/auth/google/start?client=desktop&dk={dk}")).await;
            assert_eq!(r.status(), 302);
            let auth = h.client.get(format!("{base}{}", r.headers().get("Location").unwrap().to_str().unwrap())).send().await.unwrap();
            let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
            let code = query(&back, "code").unwrap();
            let state = query(&back, "state").unwrap();
            let cb = h
                .client
                .get(format!("{base}/auth/google/callback"))
                .query(&[("state", state.as_str()), ("code", code.as_str())])
                .header("Cookie", cookie_header(&r, OAUTH).unwrap())
                .send()
                .await
                .unwrap();
            let deep = cb.headers().get("Location").unwrap().to_str().unwrap().to_string();
            deep.rsplit('=').next().unwrap().to_string()
        }

        // Wrong key burns the code: the right key afterwards is also 401.
        let c1 = desktop_flow(&h, &base, GOOD_KEY).await;
        let wrong = post_json(
            &base,
            &h,
            "/auth/desktop/exchange",
            json!({"code": c1, "desktop_key": other_key()}),
        )
        .await;
        assert_eq!(wrong.status(), 401);
        let right_after = post_json(
            &base,
            &h,
            "/auth/desktop/exchange",
            json!({"code": c1, "desktop_key": GOOD_KEY}),
        )
        .await;
        assert_eq!(right_after.status(), 401, "burned by the wrong-key attempt (D-015)");
        assert_eq!(db.count("desktop_auth_codes").await, Ok(0), "code row gone");

        // The happy path: exchange 200 with a fresh session; second exchange 401.
        let c2 = desktop_flow(&h, &base, GOOD_KEY).await;
        let ok = post_json(
            &base,
            &h,
            "/auth/desktop/exchange",
            json!({"code": c2, "desktop_key": GOOD_KEY}),
        )
        .await;
        assert_eq!(ok.status(), 200, "{}", ok.text().await.unwrap_or_default());
        assert!(cookie_value(&ok, SESSION).is_some(), "fresh session cookie");
        assert_eq!(db.count("desktop_auth_codes").await, Ok(0));
        // The placeholder of THIS code is gone (S9) and a fresh session exists; the earlier
        // refused code's placeholder is still alive until it expires (the sweep cascades it).
        assert_eq!(db.count("sessions").await, Ok(2));
        let again = post_json(
            &base,
            &h,
            "/auth/desktop/exchange",
            json!({"code": c2, "desktop_key": GOOD_KEY}),
        )
        .await;
        assert_eq!(again.status(), 401, "second exchange: code reused");

        // Two concurrent exchanges of one code: exactly one 200.
        let c3 = desktop_flow(&h, &base, GOOD_KEY).await;
        let body = json!({"code": c3, "desktop_key": GOOD_KEY});
        let (t1, t2) = {
            let client = h.client.clone();
            let origin = h.origin.clone();
            let url1 = format!("{base}/auth/desktop/exchange");
            let url2 = url1.clone();
            let b1 = body.clone();
            let b2 = body.clone();
            let t1 = tokio::spawn(async move {
                client.post(&url1).header("Origin", origin).json(&b1).send().await.unwrap()
            });
            let client = h.client.clone();
            let origin = h.origin.clone();
            let t2 = tokio::spawn(async move {
                client.post(&url2).header("Origin", origin).json(&b2).send().await.unwrap()
            });
            (t1, t2)
        };
        let statuses = [t1.await.unwrap().status().as_u16(), t2.await.unwrap().status().as_u16()];
        assert_eq!(statuses.iter().filter(|s| **s == 200).count(), 1, "{statuses:?}");
        server.abort();
    }
);

// --- AC-6: sessions slide, end, and delete cascades ---------------------------------

engines!(
    fn session_slides_at_24h_and_expires_after_30_days_idle(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = sign_in(&base, &h, "g-1").await;
        let cookie = cookie_header(&r, SESSION).unwrap();

        let row = || async {
            let (expires, seen): (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) =
                sqlx::query_as("SELECT expires_at, last_seen_at FROM sessions")
                    .fetch_one(&db.pool().await)
                    .await
                    .unwrap();
            (expires, seen)
        };
        let (exp0, seen0) = row().await;

        // +23 h: valid, no Set-Cookie, row unchanged.
        h.clock.advance(chrono::Duration::hours(23));
        let me = h
            .client
            .get(format!("{base}/auth/me"))
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), 200);
        assert!(set_cookies(&me).is_empty(), "no slide before 24 h");
        let (exp1, seen1) = row().await;
        assert_eq!(exp1, exp0);
        assert_eq!(seen1, seen0);

        // +24 h 1 s from sign-in: exactly one Set-Cookie, the row moved.
        h.clock.advance(chrono::Duration::seconds(3600 + 1));
        let me = h
            .client
            .get(format!("{base}/auth/me"))
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), 200);
        let slid = set_cookies(&me);
        assert_eq!(slid.len(), 1, "exactly one slide cookie: {slid:?}");
        assert!(slid[0].starts_with(SESSION));
        assert!(slid[0].contains("Max-Age=2592000"));
        let (exp2, seen2) = row().await;
        assert!(exp2 > exp0, "expires_at moved");
        assert!(seen2 > seen0, "last_seen_at moved");

        // 30 days idle after the last slide: signed out.
        h.clock.advance(chrono::Duration::days(30) + chrono::Duration::seconds(1));
        let me = h
            .client
            .get(format!("{base}/auth/me"))
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(me.status(), 200);
        let body = body_json(me).await;
        assert_eq!(body["user"], serde_json::Value::Null, "expired: signed out");
        server.abort();
    }
);

engines!(
    fn logout_and_delete_cascade(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let r = sign_in(&base, &h, "g-1").await;
        let cookie = cookie_header(&r, SESSION).unwrap();

        // Logout: 204, Max-Age=0, row gone; a second logout is still 204.
        let out = h
            .client
            .post(format!("{base}/auth/logout"))
            .header("Origin", &h.origin)
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(out.status(), 204);
        assert!(set_cookies(&out).iter().any(|c| c.starts_with(&format!("{SESSION}=;")) && c.contains("Max-Age=0")));
        assert_eq!(db.count("sessions").await, Ok(0));
        let out2 = h
            .client
            .post(format!("{base}/auth/logout"))
            .header("Origin", &h.origin)
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(out2.status(), 204, "idempotent");

        // Delete account: 204, every table 0 for that user, the old cookie reads signed out.
        let r = sign_in(&base, &h, "g-2").await;
        let cookie = cookie_header(&r, SESSION).unwrap();
        let del = h
            .client
            .delete(format!("{base}/api/me"))
            .header("Origin", &h.origin)
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(del.status(), 204);
        // g-2's rows go by cascade; g-1 (signed out earlier, never deleted) remains.
        assert_eq!(db.count("users").await, Ok(1), "only the earlier user remains");
        assert_eq!(db.count("identities").await, Ok(1), "g-1's identity remains");
        assert_eq!(db.count("sessions").await, Ok(0), "g-2's session cascaded away");
        assert_eq!(db.count("desktop_auth_codes").await, Ok(0));
        let me = h
            .client
            .get(format!("{base}/auth/me"))
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(body_json(me).await["user"], serde_json::Value::Null, "old cookie reads signed out");

        // DELETE /api/me signed out is 401 sign-in required.
        let anon = h.client.delete(format!("{base}/api/me")).header("Origin", &h.origin).send().await.unwrap();
        assert_eq!(anon.status(), 401);
        assert_eq!(body_json(anon).await["error"], "sign-in required");
        server.abort();
    }
);

engines!(
    fn sweep_removes_expired_sessions_and_their_codes(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        // A session (sign-in) and a desktop code (placeholder + code row).
        let r = sign_in(&base, &h, "g-1").await;
        let _cookie = cookie_header(&r, SESSION).unwrap();
        let dr = get(&base, &h, &format!("/auth/google/start?client=desktop&dk={GOOD_KEY}")).await;
        let auth = h.client.get(format!("{base}{}", dr.headers().get("Location").unwrap().to_str().unwrap())).send().await.unwrap();
        let back = auth.headers().get("Location").unwrap().to_str().unwrap().to_string();
        let cb = h
            .client
            .get(format!("{base}/auth/google/callback"))
            .query(&[
                ("state", query(&back, "state").unwrap().as_str()),
                ("code", query(&back, "code").unwrap().as_str()),
            ])
            .header("Cookie", cookie_header(&dr, OAUTH).unwrap())
            .send()
            .await
            .unwrap();
        assert_eq!(cb.status(), 302);
        let landing = cb.headers().get("Location").unwrap().to_str().unwrap();
        assert!(
            landing.starts_with("vtradingview://auth?code="),
            "the desktop landing: {landing}"
        );
        assert_eq!(db.count("sessions").await, Ok(2));
        assert_eq!(db.count("desktop_auth_codes").await, Ok(1));

        // Advance past everything, sweep: both sessions go, the code by cascade.
        h.clock.advance(chrono::Duration::days(31));
        let removed = h.state.auth.as_ref().unwrap().sweep_expired().await.unwrap();
        assert_eq!(removed, 2);
        assert_eq!(db.count("sessions").await, Ok(0));
        assert_eq!(db.count("desktop_auth_codes").await, Ok(0), "codes die by cascade");
        server.abort();
    }
);

// --- AC-7 (auth half): hosted mode, the log ring keeps no secret --------------------

engines!(
    fn log_ring_holds_no_secret_after_full_flows(db) {
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        // Without this the ring is empty and the scan below proves nothing (found by AC-8).
        let ring = hosted::feed_ring(&h);
        let (base, server) = h.serve().await;

        // A full set of flows, including refusals.
        let token = hosted::one_tap_token(&h, "g-1");
        let _ = post_json(&base, &h, "/auth/google/onetap", json!({"credential": token})).await;
        let forged = format!("forged-credential-{}", "x".repeat(40));
        let _ = post_json(&base, &h, "/auth/google/onetap", json!({"credential": forged})).await;
        let _ = get(&base, &h, "/auth/google/start?client=web").await;
        let c = hosted::sign_in(&base, &h, "g-2").await;
        let cookie = cookie_header(&c, SESSION).unwrap();
        let _ = h.client.post(format!("{base}/auth/logout")).header("Origin", &h.origin).header("Cookie", cookie.as_str()).send().await.unwrap();
        let _ = get(&base, &h, &format!("/auth/google/start?client=desktop&dk={GOOD_KEY}")).await;
        let bad_dk = get(&base, &h, "/auth/google/start?client=desktop&dk=nope").await;
        assert_eq!(bad_dk.status(), 400);

        // The ring (every log line the server kept) holds no secret value.
        let snapshot = ring.snapshot(0, false);
        let rows = snapshot.lines.unwrap_or_default();
        assert!(
            rows.iter().any(|r| r.msg.contains("refusal")),
            "the ring captured the refusal lines (otherwise this test proves nothing): {rows:?}"
        );
        let lines = format!("{rows:?}");
        assert!(!lines.contains(&token), "the id token is never logged: {lines}");
        assert!(!lines.contains(&cookie), "no session cookie value in the ring");
        // A credential that DID get logged shows as `[redacted]` (the ring's redactor masks
        // `token=...` and any 40+ char base64url run): the value itself never survives.
        assert!(
            !lines.contains(&forged),
            "the presented credential is never logged: {lines}"
        );
        // And the refusal was actually logged (the reason only), or the scan is vacuous.
        assert!(lines.contains("refusal"), "the refusal lines reached the ring");
        // The contract is stricter than "redaction saved us": the refusal line carries the
        // variant and nothing else. A credential field, even one the redactor masks, fails.
        for row in &rows {
            if row.msg.contains("refusal") {
                assert!(
                    !row.msg.contains("token=") && !row.msg.contains("credential="),
                    "a refusal line must not carry a credential field: {}",
                    row.msg
                );
            }
        }
        assert!(!lines.contains("g-2@example.test"), "no email in the ring");
        server.abort();
    }
);

// --- AC-7: the mounted /__test-idp router in hosted mode only -----------------------

engines!(
    fn test_idp_router_serves_the_flow_over_http(db) {
        // The memory path never reads /__test-idp; this proves the mount answers and that a
        // minted credential (with the configured aud) verifies end to end.
        let h = hosted::Harness::hosted(db, &[], "http://127.0.0.1:8770", IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let mint = post_json(&base, &h, "/__test-idp/mint", json!({"sub": "g-web"})).await;
        assert_eq!(mint.status(), 200);
        let credential = body_json(mint).await["credential"].as_str().unwrap().to_string();
        let r = post_json(&base, &h, "/auth/google/onetap", json!({"credential": credential})).await;
        assert_eq!(r.status(), 200, "the minted credential signs in");
        assert_eq!(db.count("users").await, Ok(1));
        server.abort();
    }
);

// --- helpers ------------------------------------------------------------------------

fn b64url(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn sha256_hex(s: &str) -> String {
    use sha2::Digest as _;
    let d = sha2::Sha256::digest(s.as_bytes());
    hex(&d)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn query(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    for pair in q.split('&') {
        let (k, v) = pair.split_once('=')?;
        if k == key {
            return Some(v.to_string());
        }
    }
    None
}
