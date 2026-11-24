//! Every account statement the server issues (S1 to S14), in one private home (architecture
//! decision 3). Statements are verbatim from `03-ARCHITECTURE.md`; every function takes only
//! hashes and typed values (decision 4), and every effect is one transaction driven by
//! [`crate::auth`].
//!
//! pgsqlite rules (D-014): one table per statement, no `RETURNING`, join, subquery, cast,
//! function or default. `rows_affected` carries the signals (S3/S4 zero = a racer won, S13
//! `== 1` = this exchange won).

use sqlx::PgPool;

use crate::auth::ids::{CodeHash, KeyHash, SessionHash};

/// S1.
const S1: &str = "SELECT user_id FROM identities WHERE provider = $1 AND subject = $2";
/// S2.
const S2: &str = "INSERT INTO users (id, email, email_verified, name, picture, created_at, last_login_at) VALUES ($1, $2, $3, $4, $5, $6, $7)";
/// S3.
const S3: &str = "INSERT INTO identities (provider, subject, user_id, created_at) VALUES ($1, $2, $3, $4) ON CONFLICT (provider, subject) DO NOTHING";
/// S4.
const S4: &str = "UPDATE users SET email = $1, email_verified = $2, name = $3, picture = $4, last_login_at = $5 WHERE id = $6";
/// S5.
const S5: &str = "INSERT INTO sessions (id_hash, user_id, created_at, expires_at, last_seen_at, user_agent) VALUES ($1, $2, $3, $4, $5, $6)";
/// S6.
const S6: &str = "SELECT user_id, expires_at, last_seen_at FROM sessions WHERE id_hash = $1";
/// S7.
const S7: &str = "SELECT id, email, email_verified, name, picture FROM users WHERE id = $1";
/// S8.
const S8: &str = "UPDATE sessions SET expires_at = $1, last_seen_at = $2 WHERE id_hash = $3";
/// S9.
const S9: &str = "DELETE FROM sessions WHERE id_hash = $1";
/// S10.
const S10: &str = "DELETE FROM users WHERE id = $1";
/// S11.
const S11: &str = "INSERT INTO desktop_auth_codes (code_hash, desktop_key_hash, session_id_hash, expires_at) VALUES ($1, $2, $3, $4)";
/// S12.
const S12: &str = "SELECT desktop_key_hash, session_id_hash, expires_at FROM desktop_auth_codes WHERE code_hash = $1";
/// S13.
const S13: &str = "DELETE FROM desktop_auth_codes WHERE code_hash = $1";
/// S14.
const S14: &str = "DELETE FROM sessions WHERE expires_at < $1";

/// Every statement this file issues, for the shape test.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] =
    &[S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14];

/// A user row as S7 reads it.
pub(crate) struct UserRow {
    pub(crate) id: uuid::Uuid,
    pub(crate) email: String,
    pub(crate) email_verified: bool,
    pub(crate) name: Option<String>,
    pub(crate) picture: Option<String>,
}

/// S6's row.
pub(crate) struct SessionRow {
    pub(crate) user_id: uuid::Uuid,
    pub(crate) expires_at: chrono::DateTime<chrono::Utc>,
    pub(crate) last_seen_at: chrono::DateTime<chrono::Utc>,
}

/// S12's row.
pub(crate) struct CodeRow {
    pub(crate) desktop_key_hash: KeyHash,
    pub(crate) session_id_hash: SessionHash,
    pub(crate) expires_at: chrono::DateTime<chrono::Utc>,
}

/// S1 over a transaction's connection (every identity read is part of an attempt).
pub(crate) async fn identity_user_tx(
    tx: &mut sqlx::PgConnection,
    provider: &str,
    subject: &str,
) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    sqlx::query_as::<_, (uuid::Uuid,)>(S1)
        .bind(provider)
        .bind(subject)
        .fetch_optional(&mut *tx)
        .await
        .map(|r| r.map(|(user_id,)| user_id))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_user_tx(
    tx: &mut sqlx::PgConnection,
    id: uuid::Uuid,
    email: &str,
    email_verified: bool,
    name: Option<&str>,
    picture: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(S2)
        .bind(id)
        .bind(email)
        .bind(email_verified)
        .bind(name)
        .bind(picture)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

/// `Ok(false)` is the racer signal (`rows_affected == 0`): another first sign-in won.
pub(crate) async fn insert_identity_tx(
    tx: &mut sqlx::PgConnection,
    provider: &str,
    subject: &str,
    user_id: uuid::Uuid,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, sqlx::Error> {
    sqlx::query(S3)
        .bind(provider)
        .bind(subject)
        .bind(user_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map(|r| r.rows_affected() > 0)
}

/// The update's `rows_affected` is not a reliable signal on pgsqlite (it reports 0 for an
/// unchanged row): the caller checks S7 after the update instead.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_user_profile_tx(
    tx: &mut sqlx::PgConnection,
    user_id: uuid::Uuid,
    email: &str,
    email_verified: bool,
    name: Option<&str>,
    picture: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    // Sequential placeholders in text order (see the S4 comment).
    sqlx::query(S4)
        .bind(email)
        .bind(email_verified)
        .bind(name)
        .bind(picture)
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

pub(crate) async fn insert_session_tx(
    tx: &mut sqlx::PgConnection,
    id_hash: &SessionHash,
    user_id: uuid::Uuid,
    expires_at: chrono::DateTime<chrono::Utc>,
    user_agent: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(S5)
        .bind(id_hash.as_bytes())
        .bind(user_id)
        .bind(now)
        .bind(expires_at)
        .bind(now)
        .bind(user_agent)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

pub(crate) async fn session_row(
    pool: &PgPool,
    id_hash: &SessionHash,
) -> Result<Option<SessionRow>, sqlx::Error> {
    sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(S6)
    .bind(id_hash.as_bytes())
    .fetch_optional(pool)
    .await
    .map(|r| {
        r.map(|(user_id, expires_at, last_seen_at)| SessionRow {
            user_id,
            expires_at,
            last_seen_at,
        })
    })
}

pub(crate) async fn user_row(
    pool: &PgPool,
    id: uuid::Uuid,
) -> Result<Option<UserRow>, sqlx::Error> {
    sqlx::query_as::<_, (uuid::Uuid, String, bool, Option<String>, Option<String>)>(S7)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map(|r| {
            r.map(|(id, email, email_verified, name, picture)| UserRow {
                id,
                email,
                email_verified,
                name,
                picture,
            })
        })
}

pub(crate) async fn slide_session(
    pool: &PgPool,
    id_hash: &SessionHash,
    expires_at: chrono::DateTime<chrono::Utc>,
    last_seen_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(S8)
        .bind(expires_at)
        .bind(last_seen_at)
        .bind(id_hash.as_bytes())
        .execute(pool)
        .await
        .map(|_| ())
}

pub(crate) async fn delete_session(
    pool: &PgPool,
    id_hash: &SessionHash,
) -> Result<(), sqlx::Error> {
    sqlx::query(S9)
        .bind(id_hash.as_bytes())
        .execute(pool)
        .await
        .map(|_| ())
}

/// S9 over a transaction's connection (the exchange's placeholder delete).
pub(crate) async fn delete_session_tx(
    tx: &mut sqlx::PgConnection,
    id_hash: &SessionHash,
) -> Result<(), sqlx::Error> {
    sqlx::query(S9)
        .bind(id_hash.as_bytes())
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

pub(crate) async fn delete_user(pool: &PgPool, id: uuid::Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(S10).bind(id).execute(pool).await.map(|_| ())
}

pub(crate) async fn insert_desktop_code(
    tx: &mut sqlx::PgConnection,
    code_hash: &CodeHash,
    key_hash: &KeyHash,
    session_id_hash: &SessionHash,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(S11)
        .bind(code_hash.as_bytes())
        .bind(key_hash.as_bytes())
        .bind(session_id_hash.as_bytes())
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map(|_| ())
}

pub(crate) async fn code_row(
    pool: &PgPool,
    code_hash: &CodeHash,
) -> Result<Option<CodeRow>, sqlx::Error> {
    sqlx::query_as::<_, (Vec<u8>, Vec<u8>, chrono::DateTime<chrono::Utc>)>(S12)
        .bind(code_hash.as_bytes())
        .fetch_optional(pool)
        .await
        .map(|r| {
            r.map(|(k, s, expires_at)| CodeRow {
                desktop_key_hash: KeyHash::from_bytes(k.try_into().expect("key hash is 32 bytes")),
                session_id_hash: SessionHash::from_bytes(
                    s.try_into().expect("session hash is 32 bytes"),
                ),
                expires_at,
            })
        })
}

/// `Ok(true)` means this exchange won (`rows_affected == 1`); `Ok(false)` is CodeReused.
pub(crate) async fn burn_code(pool: &PgPool, code_hash: &CodeHash) -> Result<bool, sqlx::Error> {
    sqlx::query(S13)
        .bind(code_hash.as_bytes())
        .execute(pool)
        .await
        .map(|r| r.rows_affected() == 1)
}

pub(crate) async fn sweep_expired(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<u64, sqlx::Error> {
    sqlx::query(S14)
        .bind(now)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sql_shape_violations;

    #[test]
    fn every_store_statement_is_single_table() {
        assert_eq!(STATEMENTS.len(), 14);
        for s in STATEMENTS {
            let v = sql_shape_violations(s);
            assert!(v.is_empty(), "{s}\n  -> {v:?}");
        }
    }
}
