//! The saved-state storage (05-01 AC-3..AC-9): the optimistic-concurrency rule, the two
//! limits, deletion scoping, and cross-user isolation — at the `Auth` method level, no HTTP.
//! The 256-key fixture is inserted with raw SQL, never through the code under test (05-01
//! Task 3); no socket is opened (L-016), so the default clock drives everything.

#[path = "common/hosted.rs"]
mod hosted;

use hosted::{db_common, IdpKind};
use hub_core::auth::{PutOutcome, KEY_MAX, VALUE_MAX};

const PUBLIC: &str = "http://127.0.0.1:8770";
const SESSION_COOKIE: &str = "__Host-vtv_session";

/// A signed-in member resolved through the real path: sign in over One Tap, take the session
/// cookie from the response, and let `Auth::resolve` produce the `Member` the gate would hand
/// a handler. (Tests cannot build `Member` directly — `session` is private by design.)
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
    fn stale_version_is_a_conflict_carrying_the_current_value(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        let put = auth.state_put(&m1, "chartHub", "\"A\"", 0).await.unwrap();
        assert_eq!(put, PutOutcome::Stored { version: 1 });

        let put = auth.state_put(&m1, "chartHub", "\"B\"", 5).await.unwrap();
        match put {
            PutOutcome::Conflict { version, value } => {
                assert_eq!(version, 1);
                assert_eq!(value, "\"A\"");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        // The refused write changed nothing.
        let rows = auth.state_all(&m1).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "\"A\"");
        assert_eq!(rows[0].version, 1);
        server.abort();
    }
);

engines!(
    fn matching_version_advances_exactly_one_step(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        auth.state_put(&m1, "chartHub", "\"A\"", 0).await.unwrap();
        let put = auth.state_put(&m1, "chartHub", "\"B\"", 1).await.unwrap();
        assert_eq!(put, PutOutcome::Stored { version: 2 });
        // Replaying the old expectation is now a conflict at the new version.
        let put = auth.state_put(&m1, "chartHub", "\"C\"", 1).await.unwrap();
        match put {
            PutOutcome::Conflict { version, value } => {
                assert_eq!(version, 2, "{value}");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        server.abort();
    }
);

engines!(
    fn insert_requires_must_not_exist_and_reports_a_lost_race(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        let put = auth.state_put(&m1, "chartHubIndicators", "\"B\"", 0).await.unwrap();
        assert_eq!(put, PutOutcome::Stored { version: 1 });
        // A second "insert" on an existing key is a conflict showing the first value.
        let put = auth.state_put(&m1, "chartHubIndicators", "\"C\"", 0).await.unwrap();
        match put {
            PutOutcome::Conflict { version, value } => {
                assert_eq!(version, 1);
                assert_eq!(value, "\"B\"");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        server.abort();
    }
);

engines!(
    fn a_value_over_value_max_is_refused_before_any_write(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        let big = "x".repeat(VALUE_MAX + 1);
        let put = auth.state_put(&m1, "chartHub", &big, 0).await.unwrap();
        assert_eq!(put, PutOutcome::TooLarge);
        assert_eq!(db.count("user_state").await.unwrap(), 0, "no row written");

        let exact = "x".repeat(VALUE_MAX);
        let put = auth.state_put(&m1, "chartHub", &exact, 0).await.unwrap();
        assert_eq!(put, PutOutcome::Stored { version: 1 }, "the boundary value fits");
        server.abort();
    }
);

engines!(
    fn the_key_cap_undoes_only_the_offending_row(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        // Fill to the cap with raw SQL — a fixture through the code under test proves nothing
        // about it (05-01 Task 3). Bound parameters, not string literals: pgsqlite parses a
        // uuid text literal differently from a bound value (the FK failure this avoids). Keys
        // stay inside the charset rule; values are tiny.
        let pool = db.pool().await;
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-20T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let stmt = String::from(
            "INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1, $2, $3, $4, $5)",
        );
        for i in 0..KEY_MAX {
            sqlx::query(&stmt)
                .bind(m1.user.id)
                .bind(format!("filler-{i:04}"))
                .bind("\"x\"")
                .bind(1_i32)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("filler {i}: {e}"));
        }
        drop(stmt);
        drop(pool);
        assert_eq!(db.count("user_state").await.unwrap(), KEY_MAX as i64);

        // The 257th key inserts, is counted, and is undone at its own version.
        let put = auth.state_put(&m1, "chartHubTheme", "\"dark\"", 0).await.unwrap();
        assert_eq!(put, PutOutcome::TooManyKeys);
        assert_eq!(
            db.count("user_state").await.unwrap(),
            KEY_MAX as i64,
            "the cap holds and the refused row is gone"
        );
        // The filler rows are untouched, and the refused key is absent.
        let rows = auth.state_all(&m1).await.unwrap();
        assert_eq!(rows.len(), KEY_MAX);
        assert!(rows.iter().all(|r| r.key.starts_with("filler-")));
        server.abort();
    }
);

engines!(
    fn delete_is_scoped_to_the_owner_and_idempotent(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;
        let m2 = member_for(&h, &base, "sub-2").await;

        auth.state_put(&m1, "chartHub", "\"one\"", 0).await.unwrap();
        auth.state_put(&m2, "chartHub", "\"two\"", 0).await.unwrap();

        auth.state_delete(&m1, "chartHub").await.unwrap();
        let rows1 = auth.state_all(&m1).await.unwrap();
        assert!(rows1.is_empty(), "sub-1's row is gone");
        let rows2 = auth.state_all(&m2).await.unwrap();
        assert_eq!(rows2.len(), 1, "sub-2's row is untouched");
        assert_eq!(rows2[0].value, "\"two\"");

        // A missing key deletes as success (removeItem is not an error condition).
        auth.state_delete(&m1, "chartHub").await.unwrap();
        auth.state_delete(&m1, "never-existed").await.unwrap();
        server.abort();
    }
);

engines!(
    fn one_user_never_reads_another_state(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;
        let m2 = member_for(&h, &base, "sub-2").await;

        auth.state_put(&m1, "chartHub", "\"mine\"", 0).await.unwrap();
        auth.state_put(&m2, "chartHub", "\"theirs\"", 0).await.unwrap();

        let rows = auth.state_all(&m1).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "\"mine\"");
        let rows = auth.state_all(&m2).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].value, "\"theirs\"");
        server.abort();
    }
);

engines!(
    fn backups_expire_only_past_the_cutoff(db) {
        let h = hosted::Harness::hosted(db, &[], PUBLIC, IdpKind::Memory).await;
        let (base, server) = h.serve().await;
        let auth = h.state.auth.as_ref().expect("hosted");
        let m1 = member_for(&h, &base, "sub-1").await;

        // Three rows at different ages, written raw: the prune must be tested against rows it
        // did not write (05-01 Task 3 / AC-8's fixture rule). Bound parameters, not literals
        // (same pgsqlite uuid-text rule as the cap fixture).
        let pool = db.pool().await;
        let old_at = chrono::DateTime::parse_from_rfc3339("2026-10-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let new_at = chrono::DateTime::parse_from_rfc3339("2026-11-24T09:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for (key, value, version, at) in [
            ("backup:chartHub:2026-10-01-120000", "\"old\"", 1_i32, old_at),
            ("backup:chartHubTheme:2026-11-24-090000", "\"new\"", 1_i32, new_at),
            ("chartHub", "\"live\"", 3_i32, old_at),
        ] {
            sqlx::query("INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1, $2, $3, $4, $5)")
                .bind(m1.user.id)
                .bind(key)
                .bind(value)
                .bind(version)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("fixture {key}: {e}"));
        }
        drop(pool);

        // 30 days before the test clock's date; the 55-day-old row goes, the 1-day-old and the
        // non-backup stay.
        let removed = auth
            .expire_backups(chrono::Utc::now() - chrono::Duration::days(30))
            .await
            .unwrap();
        assert_eq!(removed, 1, "only the 55-day-old backup");
        let rows = auth.state_all(&m1).await.unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"backup:chartHubTheme:2026-11-24-090000"));
        assert!(keys.contains(&"chartHub"), "a non-backup key is never pruned, however old");
        assert!(!keys.contains(&"backup:chartHub:2026-10-01-120000"));
        server.abort();
    }
);
