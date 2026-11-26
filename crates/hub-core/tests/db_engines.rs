//! The migration runner and the R3 schema on real engines (03-01): pgsqlite always (the owner's
//! binary, the stricter engine), Postgres 17 when `VTV_TEST_PG_URL` is set (CI).

#[path = "common/db.rs"]
mod db_common;

use chrono::{DateTime, Utc};
use hub_core::db::{is_unique_violation, Db, DbError, Migration, MIGRATIONS};
use sqlx::Row;
use std::time::{Duration, Instant};
use uuid::Uuid;

async fn versions(db: &db_common::TestDb) -> Vec<i64> {
    let pool = db.pool().await;
    let mut v: Vec<i64> = sqlx::query_scalar("SELECT version FROM schema_migrations")
        .fetch_all(&pool)
        .await
        .expect("schema_migrations");
    v.sort();
    v
}

const USERS_ONLY: &[Migration] = MIGRATIONS;

static EDITED: [Migration; 1] = [Migration {
    version: 1,
    name: "users",
    up: "CREATE TABLE users (id uuid PRIMARY KEY);",
    down: "DROP TABLE users;",
}];

static NONE: [Migration; 0] = [];

/// A file beyond the applied set: versions 1 to 4 applied, then a failing version 5 (0003 applied
/// since 07-01 and 0004 since 09-01, so the failing probe sits one past the real list).
static FAILING_THIRD: [Migration; 5] = [
    Migration {
        version: 1,
        name: "users",
        up: include_str!("../migrations/0001_users.up.sql"),
        down: include_str!("../migrations/0001_users.down.sql"),
    },
    Migration {
        version: 2,
        name: "user_state",
        up: include_str!("../migrations/0002_user_state.up.sql"),
        down: include_str!("../migrations/0002_user_state.down.sql"),
    },
    Migration {
        version: 3,
        name: "mt5_bridges",
        up: include_str!("../migrations/0003_mt5_bridges.up.sql"),
        down: include_str!("../migrations/0003_mt5_bridges.down.sql"),
    },
    Migration {
        version: 4,
        name: "pairing",
        up: include_str!("../migrations/0004_pairing.up.sql"),
        down: include_str!("../migrations/0004_pairing.down.sql"),
    },
    Migration {
        version: 5,
        name: "broken",
        // The first statement succeeds, the second fails: nothing of version 5 may remain.
        up: "CREATE TABLE t_partial (x bigint PRIMARY KEY);\nCREATE TABLE t_partial (x bigint PRIMARY KEY);",
        down: "DROP TABLE t_partial;",
    },
];

engines!(
    fn up_up_down_up(db) {
        let url = db.secret();
        Db::open(&url).await.expect("first open");
        // Every embedded migration applies on the first open (0003 joined in 07-01, 0004 in 09-01).
        assert_eq!(versions(db).await, vec![1, 2, 3, 4], "{}: first open", db.engine.name());
        let second = Db::open(&url).await.expect("second open applies nothing");
        assert_eq!(versions(db).await, vec![1, 2, 3, 4], "{}: second open", db.engine.name());
        second.migrate_down_all().await.expect("down all");
        assert_eq!(db.count("schema_migrations").await, Ok(0));
        for t in ["users", "identities", "sessions", "desktop_auth_codes", "user_state", "mt5_bridges", "pairing_codes", "mt5_bridge_secrets"] {
            assert!(db.count(t).await.is_err(), "{}: {t} still there after down", db.engine.name());
        }
        Db::open(&url).await.expect("up again");
        assert_eq!(versions(db).await, vec![1, 2, 3, 4], "{}: up again", db.engine.name());
        assert_eq!(db.count("users").await, Ok(0));
        assert_eq!(db.count("user_state").await, Ok(0));
        assert_eq!(db.count("mt5_bridges").await, Ok(0));
        assert_eq!(db.count("pairing_codes").await, Ok(0));
        assert_eq!(db.count("mt5_bridge_secrets").await, Ok(0));
    }
);

engines!(
    fn refuses_edited_unknown_and_partial(db) {
        let url = db.secret();
        Db::open(&url).await.expect("open");
        assert!(matches!(
            Db::open_with(&url, &EDITED).await,
            Err(DbError::ChecksumChanged { version: 1 })
        ));
        assert!(matches!(
            Db::open_with(&url, &NONE).await,
            Err(DbError::UnknownApplied { version: 1 })
        ));
        match Db::open_with(&url, &FAILING_THIRD).await {
            Err(DbError::Migration { version: 5, .. }) => {}
            other => panic!("{}: expected Migration {{ version: 5 }}, got {:?}", db.engine.name(), other.err()),
        }
        assert_eq!(versions(db).await, vec![1, 2, 3, 4], "{}: no row for the failed file", db.engine.name());
        assert!(
            db.count("t_partial").await.is_err(),
            "{}: the first statement of the failed file was kept",
            db.engine.name()
        );
        // The list of this binary is still accepted afterwards.
        Db::open_with(&url, USERS_ONLY).await.expect("reopen");
    }
);

engines!(
    fn absent_table_is_detected_but_other_errors_are_not(db) {
        // Fresh database: schema_migrations is absent and gets created (no IF NOT EXISTS).
        Db::open(&db.secret()).await.expect("open on an empty database");
        let pool = db.pool().await;
        // A unique violation is a different error, never "absent".
        let uid = Uuid::now_v7();
        let at: DateTime<Utc> = "2026-11-24T10:00:00Z".parse().unwrap();
        sqlx::query("INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)")
            .bind(uid).bind("a@x.io").bind(true).bind(None::<String>).bind(None::<String>).bind(at)
            .execute(&pool).await.expect("user");
        sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
            .bind("google").bind("sub-1").bind(uid).bind(at)
            .execute(&pool).await.expect("identity");
        let dup = sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
            .bind("google").bind("sub-1").bind(uid).bind(at)
            .execute(&pool).await.expect_err("duplicate identity must fail");
        assert!(is_unique_violation(&dup), "{}: not a unique violation: {dup}", db.engine.name());
        let missing = sqlx::query("SELECT version FROM no_such_table_here")
            .fetch_all(&pool).await.expect_err("absent table");
        assert!(!is_unique_violation(&missing));
    }
);

engines!(
    fn representative_rows_round_trip(db) {
        Db::open(&db.secret()).await.expect("open");
        // Raw test SQL (not the code under test) with bound values; NULL in every nullable column.
        // Bound, not literal: pgsqlite stores a timestamptz text literal as text, which no binary
        // decode can read back (measured 2026-11-24), so nothing may write times as literals.
        let pool = db.pool().await;
        let uid: Uuid = "0193f2a4-7c1e-7b2a-9c3d-111111111111".parse().unwrap();
        let t = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        sqlx::query("INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(uid).bind("a@x.io").bind(true).bind(None::<String>).bind(None::<String>)
            .bind(t("2026-11-24T10:00:00.123456Z")).bind(t("2026-11-24T10:00:01.000001Z"))
            .execute(&pool).await.expect("user row");
        sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
            .bind("google").bind("sub-1").bind(uid).bind(t("2026-11-24T10:00:00Z"))
            .execute(&pool).await.expect("identity row");
        sqlx::query("INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(vec![0xaa_u8; 32]).bind(uid).bind(t("2026-11-24T10:00:00Z")).bind(t("2026-12-24T10:00:00Z"))
            .bind(t("2026-11-24T10:00:00Z")).bind(None::<String>)
            .execute(&pool).await.expect("session row");
        sqlx::query("INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)")
            .bind(vec![0xbb_u8; 32]).bind(vec![0xcc_u8]).bind(vec![0xaa_u8; 32]).bind(t("2026-11-24T10:01:00Z"))
            .execute(&pool).await.expect("code row");
        let name = db.engine.name();
        let u = sqlx::query("SELECT id, email, email_verified, name, picture, created_at, last_login_at FROM users")
            .fetch_one(&pool).await.expect("users row");
        assert_eq!(u.get::<Uuid, _>(0).to_string(), "0193f2a4-7c1e-7b2a-9c3d-111111111111", "{name}");
        assert_eq!(u.get::<String, _>(1), "a@x.io");
        assert!(u.get::<bool, _>(2));
        assert_eq!(u.get::<Option<String>, _>(3), None);
        assert_eq!(u.get::<Option<String>, _>(4), None);
        let created: DateTime<Utc> = u.get(5);
        assert_eq!(created.to_rfc3339(), "2026-11-24T10:00:00.123456+00:00", "{name}: microseconds kept");
        let login: DateTime<Utc> = u.get(6);
        assert_eq!(login.timestamp_subsec_micros(), 1, "{name}");
        let s = sqlx::query("SELECT id_hash, user_id, expires_at, user_agent FROM sessions")
            .fetch_one(&pool).await.expect("sessions row");
        assert_eq!(s.get::<Vec<u8>, _>(0), vec![0xaa; 32], "{name}: bytea");
        assert_eq!(s.get::<Option<String>, _>(3), None);
        let c = sqlx::query("SELECT desktop_key_hash, expires_at FROM desktop_auth_codes")
            .fetch_one(&pool).await.expect("code row");
        assert_eq!(c.get::<Vec<u8>, _>(0), vec![0xcc]);
        let i = sqlx::query("SELECT provider, subject, user_id FROM identities")
            .fetch_one(&pool).await.expect("identity row");
        assert_eq!(i.get::<String, _>(1), "sub-1");
        // Typed binds round-trip too, with microsecond times as Rust writes them.
        let t = hub_core::db::now_micros();
        let uid = Uuid::now_v7();
        sqlx::query("INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)")
            .bind(uid).bind("b@x.io").bind(false).bind(Some("B")).bind(None::<String>).bind(t)
            .execute(&pool).await.expect("bound insert");
        let back: (Uuid, bool, Option<String>, DateTime<Utc>) =
            sqlx::query_as("SELECT id, email_verified, name, created_at FROM users WHERE id = $1")
                .bind(uid).fetch_one(&pool).await.expect("bound read");
        assert_eq!(back, (uid, false, Some("B".to_string()), t), "{name}");
    }
);

engines!(
    fn constraints_and_cascades(db) {
        Db::open(&db.secret()).await.expect("open");
        let pool = db.pool().await;
        let name = db.engine.name();
        let at = hub_core::db::now_micros();
        let uid = Uuid::now_v7();
        // Foreign key: an identity for an unknown user is refused.
        let orphan = sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
            .bind("google").bind("nobody").bind(Uuid::now_v7()).bind(at)
            .execute(&pool).await;
        assert!(orphan.is_err(), "{name}: foreign key not enforced");
        sqlx::query("INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)")
            .bind(uid).bind("a@x.io").bind(true).bind(None::<String>).bind(None::<String>).bind(at)
            .execute(&pool).await.expect("user");
        sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
            .bind("google").bind("sub-1").bind(uid).bind(at)
            .execute(&pool).await.expect("identity");
        let sid = vec![1u8; 32];
        sqlx::query("INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $3, $5)")
            .bind(&sid).bind(uid).bind(at).bind(at + chrono::Duration::days(30)).bind(None::<String>)
            .execute(&pool).await.expect("session");
        sqlx::query("INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)")
            .bind(vec![2u8; 32]).bind(vec![3u8; 32]).bind(&sid).bind(at + chrono::Duration::seconds(60))
            .execute(&pool).await.expect("code");
        // Deleting the user removes everything that belongs to it.
        let gone = sqlx::query("DELETE FROM users WHERE id = $1").bind(uid).execute(&pool).await.expect("delete");
        assert_eq!(gone.rows_affected(), 1);
        for t in ["identities", "sessions", "desktop_auth_codes"] {
            assert_eq!(db.count(t).await, Ok(0), "{name}: {t} not cascaded");
        }
    }
);

engines!(
    fn a_failed_statement_does_not_poison_later_writes(db) {
        // pgsqlite desynchronizes a connection after an error (D-018): without the pool guard, the
        // next statements on that connection report success and write nothing, or fail.
        Db::open(&db.secret()).await.expect("open");
        let pool = hub_core::db::pool_options().max_connections(1).connect(&db.url).await.expect("pool of 1");
        let name = db.engine.name();
        let at = hub_core::db::now_micros();
        let ins = "INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)";
        for round in 0..3 {
            let orphan = sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
                .bind("google").bind(format!("nobody-{round}")).bind(Uuid::now_v7()).bind(at)
                .execute(&pool).await;
            assert!(orphan.is_err(), "{name}: foreign key not enforced");
            let r = sqlx::query(ins).bind(Uuid::now_v7()).bind("a@x.io").bind(true).bind(None::<String>).bind(None::<String>).bind(at)
                .execute(&pool).await.unwrap_or_else(|e| panic!("{name}: write after an error failed: {e}"));
            assert_eq!(r.rows_affected(), 1, "{name}: write after an error reported {} rows", r.rows_affected());
        }
        // An error inside a transaction: the transaction is rolled back, the next write lands.
        let uid = Uuid::now_v7();
        {
            let mut tx = pool.begin().await.expect("begin");
            sqlx::query(ins).bind(uid).bind("t@x.io").bind(true).bind(None::<String>).bind(None::<String>).bind(at)
                .execute(&mut *tx).await.expect("insert in tx");
            let bad = sqlx::query("INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4)")
                .bind("google").bind("nobody-tx").bind(Uuid::now_v7()).bind(at)
                .execute(&mut *tx).await;
            assert!(bad.is_err());
        }
        let r = sqlx::query(ins).bind(Uuid::now_v7()).bind("c@x.io").bind(true).bind(None::<String>).bind(None::<String>).bind(at)
            .execute(&pool).await.unwrap_or_else(|e| panic!("{name}: write after a failed transaction: {e}"));
        assert_eq!(r.rows_affected(), 1);
        assert_eq!(db.count("users").await, Ok(4), "{name}: 3 rounds + 1, the rolled-back row absent");
        let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1").bind(uid).fetch_one(&pool).await.unwrap();
        assert_eq!(kept, 0, "{name}: the failed transaction's row was kept");
    }
);

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_files_stay_in_the_temp_dir() {
    let Some(db) = db_common::TestDb::start(db_common::Engine::Pgsqlite).await else {
        assert!(
            !db_common::Engine::Pgsqlite.required(),
            "pgsqlite is required but absent"
        );
        eprintln!("SKIPPED pgsqlite in test_files_stay_in_the_temp_dir");
        return;
    };
    Db::open(&db.secret()).await.expect("open");
    let file = db.file.clone().expect("pgsqlite file");
    assert!(
        file.starts_with(std::env::temp_dir()),
        "{} is outside the temp dir",
        file.display()
    );
    assert!(!file.starts_with(env!("CARGO_MANIFEST_DIR")));
    assert!(file.exists(), "pgsqlite wrote no database file");
    drop(db);
    assert!(!file.exists(), "{} left behind", file.display());
}

engines!(
    #[ignore = "run only as the child of a_required_engine_that_is_absent_fails"]
    fn child_engine_probe(db) {
        let _ = db;
    }
);

#[test]
fn a_required_engine_that_is_absent_fails() {
    // Child process: the engines! macro with an impossible binary and the engine required.
    let exe = std::env::current_exe().unwrap();
    let run = |require: &str| {
        std::process::Command::new(&exe)
            .args(["child_engine_probe", "--exact", "--ignored", "--nocapture"])
            .env("PGSQLITE_BIN", "Z:/no/such/pgsqlite")
            .env_remove("VTV_TEST_PG_URL")
            .env("VTV_TEST_REQUIRE", require)
            .output()
            .unwrap()
    };
    let required = run("pgsqlite");
    assert!(
        !required.status.success(),
        "the child passed although the required engine was absent"
    );
    let text = String::from_utf8_lossy(&required.stdout).to_string()
        + &String::from_utf8_lossy(&required.stderr);
    assert!(text.contains("pgsqlite is required"), "{text}");
    let optional = run("");
    assert!(
        optional.status.success(),
        "an absent optional engine must be SKIPPED, not fail"
    );
    let text = String::from_utf8_lossy(&optional.stdout).to_string()
        + &String::from_utf8_lossy(&optional.stderr);
    assert!(text.contains("SKIPPED pgsqlite"), "{text}");
}

engines!(
    fn a_malformed_migrations_table_is_an_error_not_absent(db) {
        // Only a missing table may lead to M1; any other M2 error must stop start-up as it is.
        db.exec("CREATE TABLE schema_migrations (version bigint PRIMARY KEY, name text NOT NULL)").await;
        match Db::open(&db.secret()).await {
            Err(DbError::Schema(e)) => {
                let t = e.to_string();
                // The read (M2) must fail on the missing column; a create (M1) attempt would fail
                // with "already exists", which also quotes the SQL text, so match the error kind.
                assert!(!t.contains("already exists"), "{}: M1 ran on a present table: {t}", db.engine.name());
                assert!(
                    t.contains("no such column") || t.contains("does not exist"),
                    "{}: expected the missing-column read error, got {t}",
                    db.engine.name()
                );
            }
            other => panic!("{}: expected Schema, got ok={}", db.engine.name(), other.is_ok()),
        }
    }
);

/// Measurement (03-01 summary), not a gate: `cargo test ... -- --ignored --nocapture measure_`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn measure_pgsqlite_start_and_concurrent_writers() {
    let mut total = Duration::ZERO;
    for _ in 0..10 {
        let t0 = Instant::now();
        let db = db_common::TestDb::start(db_common::Engine::Pgsqlite)
            .await
            .expect("pgsqlite");
        total += t0.elapsed();
        drop(db);
    }
    println!(
        "MEASURE pgsqlite start (spawn to first connection), mean of 10: {:?}",
        total / 10
    );

    let db = db_common::TestDb::start(db_common::Engine::Pgsqlite)
        .await
        .expect("pgsqlite");
    let t0 = Instant::now();
    Db::open(&db.secret()).await.expect("open");
    println!(
        "MEASURE Db::open with migration 0001 on a fresh file: {:?}",
        t0.elapsed()
    );
    let pool = db.pool().await;
    let at = hub_core::db::now_micros();
    let mut a = pool.begin().await.unwrap();
    let mut b = pool.begin().await.unwrap();
    let ins = "INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $6)";
    let ra = sqlx::query(ins)
        .bind(Uuid::now_v7())
        .bind("a@x.io")
        .bind(true)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(at)
        .execute(&mut *a)
        .await;
    let t1 = Instant::now();
    let rb = tokio::time::timeout(
        Duration::from_secs(15),
        sqlx::query(ins)
            .bind(Uuid::now_v7())
            .bind("b@x.io")
            .bind(true)
            .bind(None::<String>)
            .bind(None::<String>)
            .bind(at)
            .execute(&mut *b),
    )
    .await;
    println!(
        "MEASURE writer A insert: {:?}",
        ra.map(|r| r.rows_affected())
    );
    match &rb {
        Ok(Ok(r)) => println!(
            "MEASURE writer B insert while A open: ok {} after {:?}",
            r.rows_affected(),
            t1.elapsed()
        ),
        Ok(Err(e)) => {
            let d = e.as_database_error();
            println!(
                "MEASURE writer B insert while A open: error after {:?}: code {:?} message {:?}",
                t1.elapsed(),
                d.and_then(|d| d.code()),
                d.map(|d| d.message().to_string())
            )
        }
        Err(_) => println!("MEASURE writer B insert while A open: still waiting after 15 s"),
    }
    let ca = a.commit().await;
    println!("MEASURE commit A: {:?}", ca.map_err(|e| e.to_string()));
    let cb = b.commit().await;
    println!("MEASURE commit B: {:?}", cb.map_err(|e| e.to_string()));
    println!("MEASURE users after both: {:?}", db.count("users").await);
}
