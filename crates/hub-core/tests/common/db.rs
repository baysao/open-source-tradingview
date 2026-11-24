//! Real database engines for tests (Phase 03). Each test gets its own database:
//!
//! - pgsqlite: one process per test on a free loopback port, over a fresh file in a temp directory
//!   under `std::env::temp_dir()` (outside the repo, L-008); killed and deleted when the guard drops.
//!   `--in-memory` is not used: it gives each pooled connection its own empty database (measured).
//! - Postgres: `VTV_TEST_PG_URL` names an admin URL; each test creates `vtv_t_<uuid>` and drops it.
//!
//! An engine that is not available prints `SKIPPED <engine>`, unless `VTV_TEST_REQUIRE` names it
//! (`pgsqlite`, `postgres`, comma separated), which turns the skip into a failure (CI).

#![allow(dead_code)] // each suite uses a part

use hub_core::config::Secret;
use sqlx::postgres::PgPool;
use sqlx::{Connection, Executor};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Pgsqlite,
    Postgres,
}

impl Engine {
    pub const ALL: [Engine; 2] = [Engine::Pgsqlite, Engine::Postgres];

    pub fn name(self) -> &'static str {
        match self {
            Engine::Pgsqlite => "pgsqlite",
            Engine::Postgres => "postgres",
        }
    }

    /// `VTV_TEST_REQUIRE` lists this engine.
    pub fn required(self) -> bool {
        std::env::var("VTV_TEST_REQUIRE")
            .map(|v| {
                v.split(',')
                    .any(|e| e.trim().eq_ignore_ascii_case(self.name()))
            })
            .unwrap_or(false)
    }
}

enum Guard {
    Lite {
        child: Child,
        dir: Option<tempfile::TempDir>,
    },
    Pg {
        admin_url: String,
        name: String,
    },
}

/// One test's database. Dropping it stops the engine (pgsqlite) or drops the database (Postgres).
pub struct TestDb {
    pub engine: Engine,
    pub url: String,
    /// The pgsqlite database file (None on Postgres).
    pub file: Option<PathBuf>,
    guard: Guard,
}

impl TestDb {
    /// Starts `engine`, or `None` when it is not available (the caller decides skip or fail).
    pub async fn start(engine: Engine) -> Option<TestDb> {
        match engine {
            Engine::Pgsqlite => start_pgsqlite().await,
            Engine::Postgres => start_postgres().await,
        }
    }

    pub fn secret(&self) -> Secret {
        Secret::new(self.url.clone())
    }

    /// A fresh pool of 4 connections for raw test SQL (never the code under test).
    pub async fn pool(&self) -> PgPool {
        hub_core::db::pool_options()
            .max_connections(4)
            .connect(&self.url)
            .await
            .unwrap_or_else(|e| panic!("{}: connect: {e}", self.engine.name()))
    }

    /// Runs raw SQL (test setup only), panicking on error.
    pub async fn exec(&self, sql: &str) {
        let pool = self.pool().await;
        sqlx::raw_sql(sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{}: {sql}: {e}", self.engine.name()));
    }

    /// `SELECT count(*) FROM <table>`, or the error text when the query fails (an absent table).
    pub async fn count(&self, table: &str) -> Result<i64, String> {
        let pool = self.pool().await;
        sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .map_err(|e| e.to_string())
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        match &mut self.guard {
            Guard::Lite { child, dir } => {
                let _ = child.kill();
                let _ = child.wait();
                // Windows keeps the file locked until the process is gone; retry the delete briefly.
                if let Some(d) = dir.take() {
                    let path = d.path().to_path_buf();
                    if d.close().is_err() {
                        for _ in 0..20 {
                            std::thread::sleep(Duration::from_millis(50));
                            if std::fs::remove_dir_all(&path).is_ok() || !path.exists() {
                                break;
                            }
                        }
                    }
                }
            }
            Guard::Pg { admin_url, name } => {
                let (admin_url, name) = (admin_url.clone(), name.clone());
                // Drop runs inside the test's runtime: use a separate thread and runtime.
                let _ = std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("runtime");
                    rt.block_on(async move {
                        if let Ok(mut c) = sqlx::PgConnection::connect(&admin_url).await {
                            let _ = c
                                .execute(
                                    format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str(),
                                )
                                .await;
                        }
                    });
                })
                .join();
            }
        }
    }
}

/// The pgsqlite binary: `PGSQLITE_BIN`, else `~/.local/bin/pgsqlite` (the owner's install, D-011).
pub fn pgsqlite_bin() -> Option<PathBuf> {
    if let Ok(b) = std::env::var("PGSQLITE_BIN") {
        let p = PathBuf::from(b);
        return p.exists().then_some(p);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let base = PathBuf::from(home).join(".local").join("bin");
    ["pgsqlite", "pgsqlite.exe"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| p.exists())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

async fn start_pgsqlite() -> Option<TestDb> {
    let bin = pgsqlite_bin()?;
    let dir = tempfile::Builder::new()
        .prefix("vtv-pgsqlite-")
        .tempdir_in(std::env::temp_dir())
        .expect("temp dir");
    let file = dir.path().join("t.db");
    // A port can be taken between the probe and the bind: retry a few times.
    for _ in 0..5 {
        let port = free_port();
        let mut child = Command::new(&bin)
            .arg("--database")
            .arg(&file)
            .arg("--port")
            .arg(port.to_string())
            .arg("--socket-dir")
            .arg(dir.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));
        let url = format!("postgres://postgres@127.0.0.1:{port}/main?sslmode=disable");
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) {
            if let Ok(Some(_)) = child.try_wait() {
                break; // exited: the port was taken
            }
            if let Ok(c) = sqlx::PgConnection::connect(&url).await {
                let _ = c.close().await;
                return Some(TestDb {
                    engine: Engine::Pgsqlite,
                    url,
                    file: Some(file),
                    guard: Guard::Lite {
                        child,
                        dir: Some(dir),
                    },
                });
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    panic!(
        "pgsqlite did not accept connections within 10 s ({})",
        bin.display()
    );
}

async fn start_postgres() -> Option<TestDb> {
    let admin_url = std::env::var("VTV_TEST_PG_URL").ok()?;
    let name = format!("vtv_t_{}", uuid::Uuid::now_v7().simple());
    let mut c = sqlx::PgConnection::connect(&admin_url)
        .await
        .unwrap_or_else(|e| panic!("VTV_TEST_PG_URL: connect: {e}"));
    c.execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap_or_else(|e| panic!("create {name}: {e}"));
    let _ = c.close().await;
    // Replace the database path segment of the admin URL.
    let (base, query) = match admin_url.split_once('?') {
        Some((b, q)) => (b.to_string(), format!("?{q}")),
        None => (admin_url.clone(), String::new()),
    };
    let root = match base.rfind('/') {
        Some(i) if i > "postgres://".len() => base[..i].to_string(),
        _ => base.clone(),
    };
    Some(TestDb {
        engine: Engine::Postgres,
        url: format!("{root}/{name}{query}"),
        file: None,
        guard: Guard::Pg { admin_url, name },
    })
}

/// Runs a test body once per engine (`db: &TestDb`). A missing engine is SKIPPED, or a failure when
/// `VTV_TEST_REQUIRE` names it.
#[macro_export]
macro_rules! engines {
    ($(#[$m:meta])* fn $name:ident($db:ident) $body:block) => {
        $(#[$m])*
        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn $name() {
            for engine in $crate::db_common::Engine::ALL {
                match $crate::db_common::TestDb::start(engine).await {
                    Some(owned) => {
                        let $db = &owned;
                        $body
                    }
                    None if engine.required() => {
                        panic!("{} is required (VTV_TEST_REQUIRE) but not available", engine.name())
                    }
                    None => eprintln!("SKIPPED {} in {}", engine.name(), stringify!($name)),
                }
            }
        }
    };
}
