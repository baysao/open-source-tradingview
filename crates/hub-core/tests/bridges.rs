//! The bridge store and migration on real engines (07-01): pgsqlite always (the owner's binary,
//! the stricter engine), Postgres 17 when `VTV_TEST_PG_URL` is set. The `engines!` harness prints
//! `SKIPPED <engine>` for an absent engine, or fails when `VTV_TEST_REQUIRE` names it (L-039:
//! a both-engines claim cites the skipped count).

#[path = "common/db.rs"]
mod db_common;

use hub_core::bridges::{crypto, store};
use hub_core::db::Db;
use uuid::Uuid;

engines!(
    fn migration_applies_and_store_roundtrips(db) {
        Db::open(&db.secret()).await.expect("open migrates 0003");
        let pool = db.pool().await;
        let user = Uuid::now_v7();
        let at: chrono::DateTime<chrono::Utc> = "2026-11-26T00:00:00Z".parse().unwrap();
        sqlx::query("INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)")
            .bind(user)
            .bind("bridge@x.io")
            .bind(true)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(at)
            .execute(&pool)
            .await
            .expect("user row");
        let key = crypto::bridge_key(b"test-secret-for-07-01");
        let enc = crypto::seal(&key, "tok-07-01");
        let id = Uuid::now_v7();
        store::insert(
            &pool,
            &store::NewBridge {
                id,
                user_id: user,
                label: "home".into(),
                url: "https://bridge.example.com:8443".into(),
                token_enc: enc.clone(),
                symbols_json: None,
                last_ok_at: None,
                last_error: None,
                created_at: at,
            },
        )
        .await
        .expect("insert");
        let list = store::list_for(&pool, user).await.expect("list");
        assert_eq!(list.len(), 1, "{}: one bridge", db.engine.name());
        assert_eq!(list[0].label, "home");
        assert_eq!(list[0].url, "https://bridge.example.com:8443");
        // A foreign user owns nothing: the caller answers 404 to both absent and foreign (G5).
        assert!(store::get_owned(&pool, Uuid::now_v7(), id)
            .await
            .unwrap()
            .is_none());
        let row = store::get_owned(&pool, user, id)
            .await
            .unwrap()
            .expect("own row");
        assert_eq!(row.token_enc, enc);
        // The stored blob opens back to the token under the same key.
        assert_eq!(crypto::open(&key, &row.token_enc).unwrap(), "tok-07-01");
        store::set_test_result(&pool, user, id, Some(r#"[{"name":"EURUSD"}]"#), Some(at), None)
            .await
            .expect("test result ok");
        let row = store::get_owned(&pool, user, id).await.unwrap().unwrap();
        assert_eq!(row.symbols_json.as_deref(), Some(r#"[{"name":"EURUSD"}]"#));
        assert!(row.last_ok_at.is_some());
        store::set_test_result(&pool, user, id, None, None, Some("the bridge did not answer in time"))
            .await
            .expect("test result failed");
        let row = store::get_owned(&pool, user, id).await.unwrap().unwrap();
        assert_eq!(
            row.last_error.as_deref(),
            Some("the bridge did not answer in time")
        );
        assert!(row.symbols_json.is_none(), "a failed test clears the cache");
        // A foreign user cannot delete; the owner can.
        assert!(!store::delete_owned(&pool, Uuid::now_v7(), id)
            .await
            .unwrap());
        assert!(store::delete_owned(&pool, user, id).await.unwrap());
        // The user FK cascades (CUJ 8): a second bridge dies with its user.
        store::insert(
            &pool,
            &store::NewBridge {
                id: Uuid::now_v7(),
                user_id: user,
                label: "second".into(),
                url: "https://b2.example.com".into(),
                token_enc: enc,
                symbols_json: None,
                last_ok_at: None,
                last_error: None,
                created_at: at,
            },
        )
        .await
        .expect("insert 2");
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user)
            .execute(&pool)
            .await
            .expect("delete user");
        assert_eq!(
            db.count("mt5_bridges").await,
            Ok(0),
            "{}: cascade deleted the bridges",
            db.engine.name()
        );
    }
);
