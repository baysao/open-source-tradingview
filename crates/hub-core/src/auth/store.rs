//! Every account statement the server issues (S1 to S14), in one private home (architecture
//! decision 3). Statements are verbatim from `03-ARCHITECTURE.md`; every function takes only
//! hashes and typed values (decision 4), and every effect is one transaction driven by
//! [`crate::auth`].
//!
//! pgsqlite rules (D-014): one table per statement, no `RETURNING`, join, subquery, cast,
//! function or default. `rows_affected` carries the signals (S3/S4 zero = a racer won, S13
//! `== 1` = this exchange won).

use sqlx::PgPool;

use crate::auth::ids::{CodeHash, KeyHash, PairingCodeHash, SessionHash};

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
/// S15: every key of one user, in key order (`state_all`; the HTTP index and the cap count).
const S15: &str =
    "SELECT key, value, version, updated_at FROM user_state WHERE user_id = $1 ORDER BY key";
/// S16: one row's value and version (`state_put`'s read, and the conflict body's source).
const S16: &str = "SELECT value, version FROM user_state WHERE user_id = $1 AND key = $2";
/// S17: insert; `ON CONFLICT DO NOTHING` makes the insert path race-safe (`rows_affected` 0 = a
/// racer inserted first).
const S17: &str = "INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (user_id, key) DO NOTHING";
/// S18: compare-and-swap update. Placeholders ascend in textual order (DEBT-012); `rows_affected`
/// is not trusted (pgsqlite reports 0 for an unchanged row) — the caller re-reads S16.
const S18: &str = "UPDATE user_state SET value = $1, version = $2, updated_at = $3 WHERE user_id = $4 AND key = $5 AND version = $6";
/// S19: delete one key (`state_delete`; idempotent — a missing row is success).
const S19: &str = "DELETE FROM user_state WHERE user_id = $1 AND key = $2";
/// S20: delete one key at a bound version — the 256-key cap's undo, bounded to the row this call
/// wrote so a concurrent pair can never remove a legitimate row.
const S20: &str = "DELETE FROM user_state WHERE user_id = $1 AND key = $2 AND version = $3";
/// S21: expire backups older than the cutoff. A two-bound range over `key` (`'backup:'` to
/// `'backup;'`, the byte after `:`) selects exactly the `backup:*` namespace with no function and
/// one table (D-014; `LIKE` would also pass the shape check, the range avoids pattern escaping).
const S21: &str = "DELETE FROM user_state WHERE updated_at < $1 AND key >= $2 AND key < $3";
/// P1: mint a pairing code bound to the minting session (09-01; 0004's table).
const P1: &str =
    "INSERT INTO pairing_codes (code_hash, session_id_hash, expires_at) VALUES ($1, $2, $3)";
/// P2: read a code's binding (the burn precedes the check, S12/S13's ordering).
const P2: &str = "SELECT session_id_hash, expires_at FROM pairing_codes WHERE code_hash = $1";
/// P3: burn a code. `rows_affected == 1` means this call won.
const P3: &str = "DELETE FROM pairing_codes WHERE code_hash = $1";
/// P4: expired pairing codes go. P1..P3 only ever delete a code that is PRESENTED, so a code
/// nobody types would otherwise sit in the table until its minting session expired (30 days) - a
/// 10-minute credential outliving itself by a month. S14 cannot cover this: it deletes expired
/// SESSIONS, and a live session's expired code is exactly the row this removes.
const P4: &str = "DELETE FROM pairing_codes WHERE expires_at < $1";

/// Every statement this file issues, for the shape test.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    S1, S2, S3, S4, S5, S6, S7, S8, S9, S10, S11, S12, S13, S14, S15, S16, S17, S18, S19, S20, S21,
    P1, P2, P3, P4,
];

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

/// S15's row.
pub struct StateRow {
    pub key: String,
    pub value: String,
    /// The `integer` column, widened at the boundary (`i64` misdecodes: INT8 vs INT4).
    pub version: i32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn state_all(
    pool: &PgPool,
    user_id: uuid::Uuid,
) -> Result<Vec<StateRow>, sqlx::Error> {
    sqlx::query_as::<_, (String, String, i32, chrono::DateTime<chrono::Utc>)>(S15)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(key, value, version, updated_at)| StateRow {
                    key,
                    value,
                    version,
                    updated_at,
                })
                .collect()
        })
}

/// S16's row (value and version only; the caller already knows the key).
pub(crate) struct StateValue {
    pub(crate) value: String,
    pub(crate) version: i32,
}

pub(crate) async fn state_get(
    pool: &PgPool,
    user_id: uuid::Uuid,
    key: &str,
) -> Result<Option<StateValue>, sqlx::Error> {
    sqlx::query_as::<_, (String, i32)>(S16)
        .bind(user_id)
        .bind(key)
        .fetch_optional(pool)
        .await
        .map(|r| r.map(|(value, version)| StateValue { value, version }))
}

/// `Ok(true)` = this insert won (`rows_affected == 1`); `Ok(false)` = a racer inserted first.
pub(crate) async fn state_insert(
    pool: &PgPool,
    user_id: uuid::Uuid,
    key: &str,
    value: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool, sqlx::Error> {
    sqlx::query(S17)
        .bind(user_id)
        .bind(key)
        .bind(value)
        .bind(1_i32)
        .bind(now)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() > 0)
}

/// The compare-and-swap update. `rows_affected` is not a reliable signal on pgsqlite (0 also
/// means "unchanged row"): the caller re-reads S16 and decides from the version.
pub(crate) async fn state_update(
    pool: &PgPool,
    user_id: uuid::Uuid,
    key: &str,
    value: &str,
    version: i32,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    // Sequential placeholders in text order (DEBT-012; see the S18 comment).
    sqlx::query(S18)
        .bind(value)
        .bind(version)
        .bind(now)
        .bind(user_id)
        .bind(key)
        .bind(version - 1)
        .execute(pool)
        .await
        .map(|_| ())
}

pub(crate) async fn state_delete(
    pool: &PgPool,
    user_id: uuid::Uuid,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(S19)
        .bind(user_id)
        .bind(key)
        .execute(pool)
        .await
        .map(|_| ())
}

/// The cap's undo: removes only the row at exactly `version`, so a concurrent writer's row is
/// untouchable by this call.
pub(crate) async fn state_delete_at(
    pool: &PgPool,
    user_id: uuid::Uuid,
    key: &str,
    version: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(S20)
        .bind(user_id)
        .bind(key)
        .bind(version)
        .execute(pool)
        .await
        .map(|_| ())
}

/// Backups older than `cutoff` (`key` in `['backup:', 'backup:')`).
pub(crate) async fn expire_backups(
    pool: &PgPool,
    cutoff: chrono::DateTime<chrono::Utc>,
) -> Result<u64, sqlx::Error> {
    sqlx::query(S21)
        .bind(cutoff)
        .bind("backup:")
        .bind("backup;")
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

/// P2's row: the session a code is bound to, and when it dies.
pub(crate) struct PairingRow {
    pub(crate) session_id_hash: SessionHash,
    pub(crate) expires_at: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn insert_pairing_code(
    pool: &PgPool,
    code_hash: &PairingCodeHash,
    session_id_hash: &SessionHash,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(P1)
        .bind(code_hash.as_bytes())
        .bind(session_id_hash.as_bytes())
        .bind(expires_at)
        .execute(pool)
        .await
        .map(|_| ())
}

/// P2. `None` for an unknown code: the caller burns first, so this read never decides alone.
pub(crate) async fn pairing_row(
    pool: &PgPool,
    code_hash: &PairingCodeHash,
) -> Result<Option<PairingRow>, sqlx::Error> {
    sqlx::query_as::<_, (Vec<u8>, chrono::DateTime<chrono::Utc>)>(P2)
        .bind(code_hash.as_bytes())
        .fetch_optional(pool)
        .await
        .map(|r| {
            r.map(|(session_id_hash, expires_at)| PairingRow {
                session_id_hash: SessionHash::from_bytes(
                    session_id_hash
                        .try_into()
                        .expect("session hash is 32 bytes"),
                ),
                expires_at,
            })
        })
}

/// P4: expired pairing codes. Returns rows removed.
pub(crate) async fn sweep_pairing_codes(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<u64, sqlx::Error> {
    sqlx::query(P4)
        .bind(now)
        .execute(pool)
        .await
        .map(|r| r.rows_affected())
}

/// P3. `Ok(true)` means this call burned the code (the desktop exchange's S13 contract).
pub(crate) async fn burn_pairing_code(
    pool: &PgPool,
    code_hash: &PairingCodeHash,
) -> Result<bool, sqlx::Error> {
    sqlx::query(P3)
        .bind(code_hash.as_bytes())
        .execute(pool)
        .await
        .map(|r| r.rows_affected() == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sql_shape_violations;

    #[test]
    fn every_store_statement_is_single_table() {
        assert_eq!(STATEMENTS.len(), 25);
        for s in STATEMENTS {
            let v = sql_shape_violations(s);
            assert!(v.is_empty(), "{s}\n  -> {v:?}");
        }
    }
}
