//! The account database (Phase 03): a Postgres-protocol pool over pgsqlite or Postgres, and our own
//! migration runner (D-011).
//!
//! ```text
//! let db = Db::open(&settings.database_url).await?;   // connects, then applies pending migrations
//! ```
//!
//! A [`Db`] value means "connected and migrated". Every statement this crate issues touches one
//! table and uses no `RETURNING`, join, subquery, cast, function or column default (D-014): pgsqlite
//! mistypes the columns of anything else. `CREATE TABLE IF NOT EXISTS` is not used either, because
//! the owner's pgsqlite drops the clause; the runner detects its own table by selecting from it.

use crate::config::Secret;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::fmt;
use std::time::Duration;

/// One embedded migration. `version` is its number (1, 2, ...), contiguous from 1.
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub up: &'static str,
    pub down: &'static str,
}

/// Every migration this binary knows, in version order.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "users",
    up: include_str!("../migrations/0001_users.up.sql"),
    down: include_str!("../migrations/0001_users.down.sql"),
}];

/// M1: created only after M2 failed with a missing table.
const M1: &str = "CREATE TABLE schema_migrations (version bigint PRIMARY KEY, name text NOT NULL, checksum bytea NOT NULL, applied_at timestamptz NOT NULL)";
/// M2: the applied set (sorted in Rust).
const M2: &str = "SELECT version, name, checksum FROM schema_migrations";
/// M4: recorded in the same transaction as the file body.
const M4: &str =
    "INSERT INTO schema_migrations (version, name, checksum, applied_at) VALUES ($1, $2, $3, $4)";
/// M5: removed in the same transaction as the down body (tests only).
#[cfg_attr(not(feature = "test-idp"), allow(dead_code))]
const M5: &str = "DELETE FROM schema_migrations WHERE version = $1";

/// Every statement `db.rs` issues besides the migration bodies (M3) and [`PROBE`], for the shape test.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[M1, M2, M4, M5];

#[derive(Debug)]
pub enum DbError {
    /// The database could not be reached.
    Connect(sqlx::Error),
    /// Reading or creating `schema_migrations` failed.
    Schema(sqlx::Error),
    /// Applying (or reverting) one migration failed; its transaction was rolled back.
    Migration { version: i64, source: sqlx::Error },
    /// An applied migration's text differs from the one in this binary.
    ChecksumChanged { version: i64 },
    /// The database holds a version this binary does not have (an older binary on a newer database).
    UnknownApplied { version: i64 },
    /// The versions are not contiguous, or a pending version is older than an applied one.
    Gap { version: i64 },
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::Connect(e) => write!(f, "database: cannot connect: {e}"),
            DbError::Schema(e) => write!(f, "database: schema_migrations: {e}"),
            DbError::Migration { version, source } => {
                let name = MIGRATIONS
                    .iter()
                    .find(|m| m.version == *version)
                    .map(|m| m.name)
                    .unwrap_or("?");
                write!(f, "migration {version} ({name}): {source}")
            }
            DbError::ChecksumChanged { version } => write!(
                f,
                "migration {version} was edited after it was applied (add a new migration instead)"
            ),
            DbError::UnknownApplied { version } => write!(
                f,
                "the database has migration {version}, which this build does not know (newer database?)"
            ),
            DbError::Gap { version } => write!(f, "migration {version} is out of order"),
        }
    }
}

impl std::error::Error for DbError {}

/// The liveness probe of [`pool_options`]: no table, so it is outside the shape rule on purpose.
const PROBE: &str = "SELECT 1";

/// Pool options every pool in this crate (and its tests) must use. pgsqlite desynchronizes a
/// connection after any statement error (both the owner's 0.0.22 and the fork, measured
/// 2026-11-24): later replies arrive one message late, so a reused statement can report success
/// and write nothing. Before a connection is handed out it must answer `SELECT 1` with 1; a
/// poisoned one is closed instead (which also rolls back a transaction it held) and replaced.
/// Postgres pays one loopback round trip per acquire. D-018.
pub fn pool_options() -> PgPoolOptions {
    PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(5))
        .test_before_acquire(false)
        .before_acquire(|conn, _meta| {
            Box::pin(async move {
                Ok(sqlx::query_scalar::<_, i32>(PROBE)
                    .fetch_one(&mut *conn)
                    .await
                    .map(|v| v == 1)
                    .unwrap_or(false))
            })
        })
}

/// A pool over a migrated database. Cheap to clone.
#[derive(Clone)]
pub struct Db(PgPool);

impl Db {
    /// Connects (pool of 5, acquire timeout 5 s), then applies every pending migration of
    /// [`MIGRATIONS`], one transaction per file. A failure re-reads the applied set once, so a second
    /// process that applied the same file meanwhile is not an error.
    pub async fn open(url: &Secret) -> Result<Db, DbError> {
        Db::open_list(url, MIGRATIONS).await
    }

    /// [`open`](Self::open) with another migration list (tests: edited, missing and failing files).
    #[cfg(feature = "test-idp")]
    pub async fn open_with(url: &Secret, list: &'static [Migration]) -> Result<Db, DbError> {
        Db::open_list(url, list).await
    }

    async fn open_list(url: &Secret, list: &'static [Migration]) -> Result<Db, DbError> {
        let pool = pool_options()
            .max_connections(5)
            .connect(url.expose())
            .await
            .map_err(DbError::Connect)?;
        let applied = ensure_table(&pool).await?;
        for m in plan_migrations(list, &applied)? {
            if let Err(source) = apply(&pool, m).await {
                // A concurrent starter may have applied it: accept only an exact match.
                let now = read_applied(&pool).await.map_err(DbError::Schema)?;
                let sum = checksum(m.up).to_vec();
                if !now.iter().any(|(v, c)| *v == m.version && *c == sum) {
                    return Err(DbError::Migration {
                        version: m.version,
                        source,
                    });
                }
            }
        }
        Ok(Db(pool))
    }

    #[allow(dead_code)] // the auth store reads through it (03-02)
    pub(crate) fn pool(&self) -> &PgPool {
        &self.0
    }

    /// Reverts every applied migration, newest first, one transaction each (tests only). Leaves an
    /// empty `schema_migrations`.
    #[cfg(feature = "test-idp")]
    pub async fn migrate_down_all(&self) -> Result<(), DbError> {
        let mut applied = read_applied(&self.0).await.map_err(DbError::Schema)?;
        applied.sort_by_key(|a| std::cmp::Reverse(a.0));
        for (version, _) in applied {
            let m = MIGRATIONS
                .iter()
                .find(|m| m.version == version)
                .ok_or(DbError::UnknownApplied { version })?;
            let run = async {
                let mut tx = self.0.begin().await?;
                sqlx::raw_sql(m.down).execute(&mut *tx).await?;
                sqlx::query(M5).bind(version).execute(&mut *tx).await?;
                tx.commit().await
            };
            run.await
                .map_err(|source| DbError::Migration { version, source })?;
        }
        Ok(())
    }
}

/// The applied set, creating `schema_migrations` when it is absent.
async fn ensure_table(pool: &PgPool) -> Result<Vec<(i64, Vec<u8>)>, DbError> {
    match read_applied(pool).await {
        Ok(v) => Ok(v),
        Err(e) if is_missing_table(&e) => {
            sqlx::query(M1)
                .execute(pool)
                .await
                .map_err(DbError::Schema)?;
            read_applied(pool).await.map_err(DbError::Schema)
        }
        Err(e) => Err(DbError::Schema(e)),
    }
}

async fn read_applied(pool: &PgPool) -> Result<Vec<(i64, Vec<u8>)>, sqlx::Error> {
    let mut rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(M2).fetch_all(pool).await?;
    rows.sort_by_key(|r| r.0);
    Ok(rows.into_iter().map(|(v, _, c)| (v, c)).collect())
}

async fn apply(pool: &PgPool, m: &Migration) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(m.up).execute(&mut *tx).await?;
    sqlx::query(M4)
        .bind(m.version)
        .bind(m.name)
        .bind(checksum(m.up).to_vec())
        .bind(now_micros())
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// Now, truncated to microseconds (what `timestamptz` keeps), so a value read back equals it.
pub fn now_micros() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp_micros(now.timestamp_micros()).unwrap_or(now)
}

/// SHA-256 of the file text with CRLF normalized to LF (a Windows working copy may be CRLF).
pub fn checksum(sql: &str) -> [u8; 32] {
    Sha256::digest(sql.replace("\r\n", "\n").as_bytes()).into()
}

/// Pure: the migrations of `all` still to apply, given the applied `(version, checksum)` set.
/// Refuses a changed checksum, an applied version `all` lacks, a non-contiguous list, and a pending
/// version older than an applied one.
pub fn plan_migrations<'m>(
    all: &'m [Migration],
    applied: &[(i64, Vec<u8>)],
) -> Result<Vec<&'m Migration>, DbError> {
    for (i, m) in all.iter().enumerate() {
        if m.version != i as i64 + 1 {
            return Err(DbError::Gap { version: m.version });
        }
    }
    for (version, sum) in applied {
        let m = all
            .iter()
            .find(|m| m.version == *version)
            .ok_or(DbError::UnknownApplied { version: *version })?;
        if checksum(m.up).as_slice() != sum.as_slice() {
            return Err(DbError::ChecksumChanged { version: *version });
        }
    }
    let newest = applied.iter().map(|a| a.0).max().unwrap_or(0);
    let pending: Vec<&Migration> = all
        .iter()
        .filter(|m| !applied.iter().any(|a| a.0 == m.version))
        .collect();
    if let Some(m) = pending.iter().find(|m| m.version < newest) {
        return Err(DbError::Gap { version: m.version });
    }
    Ok(pending)
}

/// A uniqueness conflict on either engine: Postgres `23505`, pgsqlite `42000` with SQLite's text.
pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error().is_some_and(|d| {
        let code = d.code();
        code.as_deref() == Some("23505")
            || (code.as_deref() == Some("42000")
                && d.message().contains("UNIQUE constraint failed"))
    })
}

/// A missing table on either engine: Postgres `42P01`, pgsqlite "no such table".
fn is_missing_table(e: &sqlx::Error) -> bool {
    e.as_database_error().is_some_and(|d| {
        d.code().as_deref() == Some("42P01") || d.message().contains("no such table")
    })
}

/// Pure: why a statement breaks the single-table rule (D-014), empty when it is fine. Comments are
/// ignored; `REFERENCES` targets in DDL are constraints, not reads, so they do not count as a table.
pub fn sql_shape_violations(statement: &str) -> Vec<String> {
    let text: String = statement
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let upper = text.to_ascii_uppercase();
    let mut out = Vec::new();
    for banned in [
        "RETURNING",
        "JOIN",
        "(SELECT",
        "::",
        "CAST(",
        "DEFAULT",
        "NOW(",
        "IF NOT EXISTS",
    ] {
        if upper.contains(banned) {
            out.push(format!("contains {banned}"));
        }
    }
    let words: Vec<&str> = upper
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',' || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    let mut tables: Vec<&str> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let after = match *w {
            "FROM" | "INTO" | "UPDATE" | "TABLE" | "ON" => words.get(i + 1),
            _ => None,
        };
        // `ON CONFLICT`, `ON DELETE CASCADE`, `DO UPDATE SET` name no table.
        if let Some(t) =
            after.filter(|t| !matches!(**t, "CONFLICT" | "DELETE" | "UPDATE" | "SET" | "NOTHING"))
        {
            if !tables.contains(t) {
                tables.push(t);
            }
        }
    }
    // `FROM a, b` is a join without the keyword.
    for part in upper.split("FROM ").skip(1) {
        let clause = part
            .split(" WHERE ")
            .next()
            .unwrap_or("")
            .split(" ORDER ")
            .next()
            .unwrap_or("");
        if clause.contains(',') {
            out.push("FROM lists more than one table".to_string());
        }
    }
    if tables.len() != 1 {
        out.push(format!("names {} tables: {tables:?}", tables.len()));
    }
    out
}

/// The statements of a migration file, split on `;` (the files hold no `;` inside a string).
pub fn split_statements(file: &str) -> Vec<String> {
    let text: String = file
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    text.split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn mig(version: i64, up: &'static str) -> Migration {
        Migration {
            version,
            name: "t",
            up,
            down: "",
        }
    }

    #[test]
    fn checksum_ignores_crlf() {
        assert_eq!(checksum("a;\r\nb;\r\n"), checksum("a;\nb;\n"));
        assert_ne!(checksum("a;\nb;\n"), checksum("a;\nc;\n"));
    }

    #[test]
    fn plan_applies_pending_in_order() {
        let all = [mig(1, "a"), mig(2, "b")];
        let none = plan_migrations(&all, &[]).unwrap();
        assert_eq!(
            none.iter().map(|m| m.version).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let one = plan_migrations(&all, &[(1, checksum("a").to_vec())]).unwrap();
        assert_eq!(one.iter().map(|m| m.version).collect::<Vec<_>>(), vec![2]);
        let both = [(1, checksum("a").to_vec()), (2, checksum("b").to_vec())];
        assert!(plan_migrations(&all, &both).unwrap().is_empty());
    }

    #[test]
    fn plan_refuses_edits_unknowns_and_gaps() {
        let all = [mig(1, "a"), mig(2, "b")];
        assert!(matches!(
            plan_migrations(&all, &[(1, checksum("edited").to_vec())]),
            Err(DbError::ChecksumChanged { version: 1 })
        ));
        assert!(matches!(
            plan_migrations(&all[..1], &[(2, checksum("b").to_vec())]),
            Err(DbError::UnknownApplied { version: 2 })
        ));
        assert!(matches!(
            plan_migrations(&[mig(1, "a"), mig(3, "c")], &[]),
            Err(DbError::Gap { version: 3 })
        ));
        assert!(matches!(
            plan_migrations(&all, &[(2, checksum("b").to_vec())]),
            Err(DbError::Gap { version: 1 })
        ));
    }

    #[test]
    fn shape_rule_flags_each_banned_form() {
        assert!(sql_shape_violations("SELECT a FROM t WHERE b = $1").is_empty());
        assert!(
            sql_shape_violations("INSERT INTO t (a) VALUES ($1) ON CONFLICT (a) DO NOTHING")
                .is_empty()
        );
        for bad in [
            "INSERT INTO t (a) VALUES ($1) RETURNING a",
            "SELECT a FROM t JOIN u ON u.x = t.x",
            "SELECT (SELECT 1 FROM u) FROM t",
            "SELECT a::text FROM t",
            "SELECT CAST(a AS text) FROM t",
            "CREATE TABLE t (a bigint DEFAULT 1)",
            "INSERT INTO t (a) VALUES (now())",
            "CREATE TABLE IF NOT EXISTS t (a bigint)",
            "SELECT a FROM t, u",
        ] {
            assert!(!sql_shape_violations(bad).is_empty(), "not flagged: {bad}");
        }
        // Two tables without a JOIN keyword are still two tables.
        assert!(!sql_shape_violations("DELETE FROM t WHERE a IN (SELECT a FROM u)").is_empty());
    }

    #[test]
    fn every_statement_and_migration_is_single_table() {
        let mut all: Vec<String> = STATEMENTS.iter().map(|s| s.to_string()).collect();
        for m in MIGRATIONS {
            all.extend(split_statements(m.up));
            all.extend(split_statements(m.down));
        }
        assert!(all.len() > 8, "split found too few statements: {all:?}");
        for s in &all {
            let v = sql_shape_violations(s);
            assert!(v.is_empty(), "{s}\n  -> {v:?}");
        }
    }
}
