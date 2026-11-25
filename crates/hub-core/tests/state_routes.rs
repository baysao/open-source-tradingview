//! The `/api/me/state` HTTP contract (05-02 AC-2..AC-9): the list omits values, a stale PUT
//! is a 409 that carries the current value and wrote nothing, the refusals are one case per
//! error string, signed-out is 401 on all four, DELETE is idempotent, the daily prune deletes
//! only old backups, and no route can name a user id other than the session's.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{db_common, IdpKind};
use serde_json::json;

const PUBLIC: &str = "http://127.0.0.1:8770";
const SESSION_COOKIE: &str = "__Host-vtv_session";

/// Sign in as `sub` and return the Cookie header value for authenticated calls.
async fn session_for(h: &hosted::Harness, base: &str, sub: &str) -> String {
    let r = hosted::sign_in(base, h, sub).await;
    assert_eq!(r.status(), 200, "sign-in {sub}");
    hosted::cookie_header(&r, SESSION_COOKIE).expect("session cookie set")
}

/// PUT with the session cookie, an If-Match header and a JSON value.
async fn put_key(
    h: &hosted::Harness,
    base: &str,
    cookie: &str,
    key: &str,
    if_match: Option<&str>,
    value: &str,
) -> reqwest::Response {
    let mut req = h
        .client
        .put(format!("{base}/api/me/state/{key}"))
        .header("Origin", &h.origin)
        .header("Cookie", cookie)
        .json(&json!({"value": value}));
    if let Some(im) = if_match {
        req = req.header("If-Match", im);
    }
    req.send().await.expect("PUT")
}

/// DELETE with the session cookie.
async fn delete_key(h: &hosted::Harness, base: &str, cookie: &str, key: &str) -> reqwest::Response {
    h.client
        .delete(format!("{base}/api/me/state/{key}"))
        .header("Origin", &h.origin)
        .header("Cookie", cookie)
        .send()
        .await
        .expect("DELETE")
}

/// GET one key with the session cookie.
async fn get_key(h: &hosted::Harness, base: &str, cookie: &str, key: &str) -> reqwest::Response {
    h.client
        .get(format!("{base}/api/me/state/{key}"))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("GET")
}

engines!(
    fn list_omits_values(db) {
        // AC-2: 200, exactly the two keys, each entry has key/version/updated_at, and the
        // serialized object carries no `value` key at all (asserted on the key set, not a
        // deserialized struct that would silently drop the field).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let r = put_key(&h, &base, &cookie, "chartHub", Some("0"), "A").await;
        assert_eq!(r.status(), 200, "insert chartHub");
        // AC-2's stated versions: chartHub at 3, chartHubTheme at 1.
        for (im, v) in [("1", "B"), ("2", "C")] {
            let r = put_key(&h, &base, &cookie, "chartHub", Some(im), v).await;
            assert_eq!(r.status(), 200, "advance chartHub to If-Match {im}");
        }
        let r = put_key(&h, &base, &cookie, "chartHubTheme", Some("0"), "dark").await;
        assert_eq!(r.status(), 200, "insert chartHubTheme");

        let r = h
            .client
            .get(format!("{base}/api/me/state"))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "list");
        let body = hosted::body_json(r).await;
        let keys = body["keys"].as_array().expect("keys array");
        assert_eq!(keys.len(), 2, "two keys");
        let names: Vec<&str> = keys.iter().map(|k| k["key"].as_str().unwrap()).collect();
        assert!(names.contains(&"chartHub") && names.contains(&"chartHubTheme"), "{names:?}");
        for k in keys {
            let fields: Vec<&str> = k.as_object().unwrap().keys().map(|s| s.as_str()).collect();
            assert_eq!(fields.len(), 3, "exactly key, version, updated_at: {fields:?}");
            assert!(fields.contains(&"key") && fields.contains(&"version") && fields.contains(&"updated_at"));
            assert!(!fields.contains(&"value"), "the list must not ship values: {fields:?}");
        }
        let version_of = |name: &str| {
            keys.iter()
                .find(|k| k["key"] == name)
                .map(|k| k["version"].as_i64().unwrap())
                .unwrap_or_else(|| panic!("{name} missing"))
        };
        assert_eq!(version_of("chartHub"), 3, "AC-2's stated version");
        assert_eq!(version_of("chartHubTheme"), 1, "AC-2's stated version");
        // `updated_at` is a string 05-03 parses: pin the format, not just the field's presence.
        for k in keys {
            let at = k["updated_at"].as_str().expect("updated_at is a string");
            chrono::DateTime::parse_from_rfc3339(at)
                .unwrap_or_else(|e| panic!("updated_at is not RFC 3339 ({at}): {e}"));
        }
        server.abort();
    }
);

engines!(
    fn stale_put_is_a_409_carrying_the_current_value_and_writes_nothing(db) {
        // AC-3: If-Match 5 at version 7 → 409 {"error":"version conflict","version":7,"value":"A"};
        // a following GET still returns "A" at 7 (a 409 that wrote would pass a status-only assert).
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        // Build up to AC-3's stated version 7 with value "A" (insert is version 1).
        assert_eq!(put_key(&h, &base, &cookie, "chartHub", Some("0"), "A").await.status(), 200);
        for im in 1..7 {
            let r = put_key(&h, &base, &cookie, "chartHub", Some(&im.to_string()), "A").await;
            assert_eq!(r.status(), 200, "advance to version {}", im + 1);
        }
        let r = put_key(&h, &base, &cookie, "chartHub", Some("5"), "B").await;
        assert_eq!(r.status(), 409, "stale If-Match");
        let body = hosted::body_json(r).await;
        assert_eq!(body["error"], "version conflict");
        assert_eq!(body["version"], 7, "the current version");
        assert_eq!(body["value"], "A", "the current value");
        let r = get_key(&h, &base, &cookie, "chartHub").await;
        let after = hosted::body_json(r).await;
        assert_eq!(after["value"], "A", "the 409 did not write");
        assert_eq!(after["version"], 7);
        server.abort();
    }
);

engines!(
    fn current_put_stores_and_advances(db) {
        // AC-4: If-Match = current → 200 {"key","version"} and the GET shows the new value.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        // AC-4's stated version 7 with value "A" (insert is version 1).
        assert_eq!(put_key(&h, &base, &cookie, "chartHub", Some("0"), "A").await.status(), 200);
        for im in 1..7 {
            let r = put_key(&h, &base, &cookie, "chartHub", Some(&im.to_string()), "A").await;
            assert_eq!(r.status(), 200, "advance to version {}", im + 1);
        }
        let r = put_key(&h, &base, &cookie, "chartHub", Some("7"), "B").await;
        assert_eq!(r.status(), 200);
        let body = hosted::body_json(r).await;
        assert_eq!(body["key"], "chartHub");
        assert_eq!(body["version"], 8, "AC-4: 7 advances to 8");
        let after = hosted::body_json(get_key(&h, &base, &cookie, "chartHub").await).await;
        assert_eq!(after["value"], "B");
        assert_eq!(after["version"], 8);
        server.abort();
    }
);

engines!(
    fn refusals_are_specific(db) {
        // AC-5: one case per error string. `too large` (the gate's 413) vs `value too large`
        // (the handler's) is the distinguishing pair — the test exists for that distinction.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;

        // No If-Match → 428 "if-match required".
        let r = h
            .client
            .put(format!("{base}/api/me/state/chartHub"))
            .header("Origin", &h.origin)
            .header("Cookie", &cookie)
            .json(&json!({"value": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 428, "missing If-Match");
        assert_eq!(hosted::body_json(r).await["error"], "if-match required");

        // Malformed If-Match → 400 "bad if-match".
        let r = put_key(&h, &base, &cookie, "chartHub", Some("abc"), "x").await;
        assert_eq!(r.status(), 400, "malformed If-Match");
        assert_eq!(hosted::body_json(r).await["error"], "bad if-match");

        // Bad key (space) → 400 "bad key".
        let r = put_key(&h, &base, &cookie, "bad key", Some("0"), "x").await;
        assert_eq!(r.status(), 400, "space in key");
        assert_eq!(hosted::body_json(r).await["error"], "bad key");

        // Bad key (`/`) → 400 "bad key". A literal `/` is a second path segment, but `%2F`
        // matches `{key}` as one segment and decodes to a slash inside the key, which is the
        // input AC-5 names.
        let r = h
            .client
            .put(format!("{base}/api/me/state/a%2Fb"))
            .header("Origin", &h.origin)
            .header("Cookie", &cookie)
            .json(&json!({"value": "x"}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "slash in key");
        assert_eq!(hosted::body_json(r).await["error"], "bad key");

        // 262_145-byte value → 413 "value too large" (the handler's string; the gate's own
        // 413 for a declared body over 300,000 says `too large`).
        let big = "x".repeat(hub_core::auth::VALUE_MAX + 1);
        let r = put_key(&h, &base, &cookie, "big", Some("0"), &big).await;
        assert_eq!(r.status(), 413, "value over VALUE_MAX");
        assert_eq!(hosted::body_json(r).await["error"], "value too large");

        // Exactly VALUE_MAX is accepted.
        let ok = "x".repeat(hub_core::auth::VALUE_MAX);
        let r = put_key(&h, &base, &cookie, "ok-big", Some("0"), &ok).await;
        assert_eq!(r.status(), 200, "value at VALUE_MAX is accepted");

        server.abort();
    }
);

engines!(
    fn too_many_keys_is_a_409(db) {
        // AC-5 (last case): the 257th key → 409 "too many keys". The 256 fillers are raw SQL
        // (05-01's rule: never through the code under test), with the user id bound — pgsqlite
        // parses a uuid text literal differently from a bound value (tests/state.rs:134), and
        // the literal form is the FK failure this avoids.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        let member = member_for(&h, &base, "sub-1").await;
        let pool = db.pool().await;
        let stmt = String::from(
            "INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1, $2, $3, $4, $5)",
        );
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-25T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        // AC-5's cap case: 256 keys exist, the next insert is refused. (The cap is corrective —
        // auth/mod.rs `state_cap_check` counts after the insert and undoes the offending row —
        // so the 257th insert is the one that answers 409.)
        for i in 0..hub_core::auth::KEY_MAX {
            sqlx::query(&stmt)
                .bind(member.user.id)
                .bind(format!("filler-{i:04}"))
                .bind("v")
                .bind(1_i32)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("filler {i}: {e}"));
        }
        let r = put_key(&h, &base, &cookie, "chartHubTheme", Some("0"), "dark").await;
        assert_eq!(r.status(), 409, "the 257th key");
        assert_eq!(hosted::body_json(r).await["error"], "too many keys");
        // The refusal is corrective: the row it wrote is undone, so the user holds 256, not 257,
        // and `chartHubTheme` is not half-written.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user_state WHERE user_id = $1")
            .bind(member.user.id)
            .fetch_one(&pool)
            .await
            .expect("count after the cap refusal");
        assert_eq!(n, hub_core::auth::KEY_MAX as i64, "the offending row was undone");
        server.abort();
    }
);

/// The `Member` for `sub`, resolved through the real path (tests cannot build `Member`
/// directly — `session` is private by design; tests/state.rs `member_for`). `resolve`
/// hashes the string it is given, so it takes the cookie's VALUE (cookie_value), not
/// the `name=value` header pair (cookie_header) — the header form hashes to a session
/// id that is not in the table and resolves as Anonymous.
async fn member_for(h: &hosted::Harness, base: &str, sub: &str) -> hub_core::auth::Member {
    let r = hosted::sign_in(base, h, sub).await;
    assert_eq!(r.status(), 200, "sign-in {sub}");
    let cookie = hosted::cookie_value(&r, SESSION_COOKIE).expect("session cookie set");
    let (viewer, _) = h
        .state
        .auth
        .as_ref()
        .expect("hosted")
        .resolve(Some(&cookie))
        .await
        .expect("the fresh session resolves");
    match viewer {
        hub_core::auth::Viewer::Member(m) => m,
        other => panic!("expected a member, got {other:?}"),
    }
}

engines!(
    fn signed_out_is_401_on_all_four(db) {
        // AC-6: every route from route_specs under /api/me/state answers 401 signed out,
        // driven from the table so a fifth route is covered without editing this test.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let specs = hub_core::routes::table::route_specs(&h.state);
        let mut checked = 0;
        for s in &specs {
            if !s.path.starts_with("/api/me/state") {
                continue;
            }
            let url = format!("{base}{}", s.path.replace("{key}", "sample"));
            let r = match &s.method {
                m if *m == reqwest::Method::GET => h.client.get(&url),
                m if *m == reqwest::Method::PUT => h.client.put(&url).json(&json!({"value": "x"})),
                m if *m == reqwest::Method::DELETE => h.client.delete(&url),
                m => panic!("unexpected method {m}"),
            }
            .header("Origin", &h.origin)
            .send()
            .await
            .unwrap();
            assert_eq!(r.status(), 401, "{} {} signed out", s.method, s.path);
            assert_eq!(hosted::body_json(r).await["error"], "sign-in required");
            checked += 1;
        }
        assert_eq!(checked, 4, "all four state routes checked");
        // AC-6's "none of them writes a row": the table is still empty afterwards.
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user_state")
            .fetch_one(&db.pool().await)
            .await
            .expect("count user_state");
        assert_eq!(n, 0, "a signed-out call wrote nothing");
        server.abort();
    }
);

engines!(
    fn delete_is_idempotent(db) {
        // AC-7: 204, key gone; the same DELETE again is still 204.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let cookie = session_for(&h, &base, "sub-1").await;
        put_key(&h, &base, &cookie, "chartHub", Some("0"), "A").await;
        let r = delete_key(&h, &base, &cookie, "chartHub").await;
        assert_eq!(r.status(), 204, "first delete");
        let r = get_key(&h, &base, &cookie, "chartHub").await;
        assert_eq!(r.status(), 404, "key gone");
        let r = delete_key(&h, &base, &cookie, "chartHub").await;
        assert_eq!(r.status(), 204, "delete is idempotent");
        server.abort();
    }
);

engines!(
    fn cross_user_read_is_impossible(db) {
        // AC-9: two sessions, one key, different values; each reads only its own. And no
        // route this plan defines accepts a user id: no path carries a `{user` template.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let u = session_for(&h, &base, "sub-1").await;
        let v = session_for(&h, &base, "sub-2").await;
        assert_eq!(put_key(&h, &base, &u, "chartHub", Some("0"), "U-value").await.status(), 200);
        assert_eq!(put_key(&h, &base, &v, "chartHub", Some("0"), "V-value").await.status(), 200);
        let u_body = hosted::body_json(get_key(&h, &base, &u, "chartHub").await).await;
        let v_body = hosted::body_json(get_key(&h, &base, &v, "chartHub").await).await;
        assert_eq!(u_body["value"], "U-value");
        assert_eq!(v_body["value"], "V-value");
        for s in hub_core::routes::table::route_specs(&h.state) {
            assert!(!s.path.contains("{user"), "no route names a user id: {}", s.path);
        }
        server.abort();
    }
);

engines!(
    fn the_prune_deletes_only_old_backups(db) {
        // AC-8: three raw rows at the stated ages; a 30-day cutoff removes only the 55-day
        // backup. Rows via raw SQL (never through the code that wrote them); the deleted one
        // asserted by count, the survivors by presence. The prune body is
        // Auth::expire_backups(cutoff) so the test names the cutoff instead of waiting a day.
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let _cookie = session_for(&h, &base, "sub-1").await;
        let member = member_for(&h, &base, "sub-1").await;
        let pool = db.pool().await;
        let stmt = String::from(
            "INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1, $2, $3, $4, $5)",
        );
        let rows = [
            // AC-8 states "55 days old": a fixed date would silently stop being 55 days old as
            // the calendar moves, so the ages are relative to now.
            ("backup:chartHub:2026-10-01-120000", chrono::Utc::now() - chrono::Duration::days(55)),
            ("backup:chartHubTheme:2026-11-24-090000", chrono::Utc::now() - chrono::Duration::days(1)),
            ("chartHub", chrono::Utc::now() - chrono::Duration::days(55)),
        ];
        for (key, at) in rows {
            sqlx::query(&stmt)
                .bind(member.user.id)
                .bind(key)
                .bind("v")
                .bind(1_i32)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("fixture {key}: {e}"));
        }

        // All three present before the prune.
        let count = |pool: &sqlx::PgPool, uid: uuid::Uuid| {
            let pool = pool.clone();
            async move {
                let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM user_state WHERE user_id = $1")
                    .bind(uid)
                    .fetch_one(&pool)
                    .await
                    .expect("count");
                n
            }
        };
        assert_eq!(count(&pool, member.user.id).await, 3, "fixtures present before the prune");

        let auth = h.state.auth.as_ref().expect("hosted");
        let cutoff = chrono::Utc::now() - chrono::Duration::days(30);
        let removed = auth.expire_backups(cutoff).await.expect("prune runs");
        assert_eq!(removed, 1, "exactly the old backup went");
        assert_eq!(count(&pool, member.user.id).await, 2, "two rows survive");

        let left: Vec<(String,)> =
            sqlx::query_as("SELECT key FROM user_state WHERE user_id = $1 ORDER BY key")
                .bind(member.user.id)
                .fetch_all(&pool)
                .await
                .expect("keys");
        let keys: Vec<&str> = left.iter().map(|(k,)| k.as_str()).collect();
        assert!(keys.contains(&"backup:chartHubTheme:2026-11-24-090000"), "{keys:?}");
        assert!(keys.contains(&"chartHub"), "{keys:?}");
        assert!(!keys.contains(&"backup:chartHub:2026-10-01-120000"), "{keys:?}");
        server.abort();
    }
);
