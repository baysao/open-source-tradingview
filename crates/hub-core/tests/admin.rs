//! The admin gate (03-02 AC-5, admin part): every `/admin/*` route answers 404 `{"error":"not
//! found"}` to everyone but a verified admin - signed out, a member, an `ADMIN_EMAILS` address
//! with `email_verified=false`, and an admin behind a foreign Origin on a non-GET. An admin
//! gets the Phase 1 logs body and the journal 200.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{body_json, db_common, get, IdpKind};
use serde_json::json;

const ADMIN: &str = "admin@example.test";

engines!(
    fn admin_routes_are_404_to_everyone_but_a_verified_admin(db) {
        let h = hosted::Harness::hosted(
            db,
            &[ADMIN],
            "http://127.0.0.1:8770",
            IdpKind::Memory,
        )
        .await;
        let (base, server) = h.serve().await;

        // Signed out: 404 on both admin routes.
        for path in ["/admin/logs", "/admin/logs/journal"] {
            let r = get(&base, &h, path).await;
            assert_eq!(r.status(), 404, "{path} signed out");
            assert_eq!(body_json(r).await["error"], "not found");
        }

        // A plain member (email not in ADMIN_EMAILS): 404.
        let token = h.idp.id_token(
            &hub_core::auth::test_idp::Claims::for_sub("g-member").aud(hosted::CLIENT_ID),
            None,
            0,
        );
        let r = h
            .client
            .post(format!("{base}/auth/google/onetap"))
            .header("Origin", &h.origin)
            .json(&json!({"credential": token}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let member_cookie = hosted::cookie_header(&r, "__Host-vtv_session").unwrap();
        let r = h
            .client
            .get(format!("{base}/admin/logs"))
            .header("Cookie", member_cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "member on /admin/logs");

        // The admin email with email_verified=false: 404 (is_admin requires the verification).
        let unverified = h.idp.id_token(
            &hub_core::auth::test_idp::Claims::for_sub("g-admin")
                .aud(hosted::CLIENT_ID)
                .email(ADMIN, false),
            None,
            0,
        );
        let r = h
            .client
            .post(format!("{base}/auth/google/onetap"))
            .header("Origin", &h.origin)
            .json(&json!({"credential": unverified}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "the unverified admin email still signs in");
        let cookie = hosted::cookie_header(&r, "__Host-vtv_session").unwrap();
        let r = h
            .client
            .get(format!("{base}/admin/logs"))
            .header("Cookie", cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "unverified admin email on /admin/logs");

        // A verified admin: 200 with the Phase 1 logs body shape, and the journal 200.
        let admin_token = h.idp.id_token(
            &hub_core::auth::test_idp::Claims::for_sub("g-admin2")
                .aud(hosted::CLIENT_ID)
                .email(ADMIN, true),
            None,
            0,
        );
        let r = h
            .client
            .post(format!("{base}/auth/google/onetap"))
            .header("Origin", &h.origin)
            .json(&json!({"credential": admin_token}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let admin_cookie = hosted::cookie_header(&r, "__Host-vtv_session").unwrap();
        let logs = h
            .client
            .get(format!("{base}/admin/logs"))
            .header("Cookie", admin_cookie.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(logs.status(), 200);
        let body = body_json(logs).await;
        assert!(body.get("boot").is_some(), "Phase 1 logs shape: {body}");
        let journal = h
            .client
            .post(format!("{base}/admin/logs/journal"))
            .header("Origin", &h.origin)
            .header("Cookie", admin_cookie.as_str())
            .json(&json!({"lines": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(journal.status(), 200, "admin journal");

        // An admin behind a foreign Origin on a non-GET: 403 (Origin rule before admin would
        // leak admin status; the gate answers 403 for the bad Origin on the POST).
        let foreign = h
            .client
            .post(format!("{base}/admin/logs/journal"))
            .header("Origin", "https://evil.example")
            .header("Cookie", admin_cookie.as_str())
            .json(&json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403, "foreign Origin on a non-GET admin route");
        server.abort();
    }
);
