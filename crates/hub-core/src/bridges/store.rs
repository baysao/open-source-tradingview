//! Every bridge statement (07-ARCHITECTURE's route table), in one home. Same rules as the auth
//! store (D-14): one table per statement, no `RETURNING`, join, subquery, cast, function or
//! default; ids and times are written by Rust; UPDATE placeholders ascend in textual order
//! (DEBT-012). Every function scopes by `user_id`, so 07-03's 404-on-foreign-id rule is the
//! statement's own shape.

use sqlx::PgPool;
use uuid::Uuid;

/// B1.
const B1: &str = "INSERT INTO mt5_bridges (id, user_id, label, url, token_enc, symbols_json, last_ok_at, last_error, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)";
/// B2: one user's bridges, oldest first (the Settings list).
const B2: &str = "SELECT id, label, url, symbols_json, last_ok_at, last_error, created_at FROM mt5_bridges WHERE user_id = $1 ORDER BY created_at, id";
/// B3: one bridge the user owns, with the sealed token (the caller opens it).
const B3: &str = "SELECT id, url, token_enc, symbols_json, last_ok_at, last_error FROM mt5_bridges WHERE id = $1 AND user_id = $2";
/// B4.
const B4: &str = "DELETE FROM mt5_bridges WHERE id = $1 AND user_id = $2";
/// B5: the result of a Test (placeholders ascend in text order, DEBT-012).
const B5: &str = "UPDATE mt5_bridges SET symbols_json = $1, last_ok_at = $2, last_error = $3 WHERE id = $4 AND user_id = $5";

/// Every statement this file issues, for the shape test.
#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[B1, B2, B3, B4, B5];

/// What the Settings list shows for one bridge: never the token.
#[derive(Debug, Clone)]
pub struct BridgeListing {
    pub id: Uuid,
    pub label: String,
    pub url: String,
    pub symbols_json: Option<String>,
    pub last_ok_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A full owned row, including the sealed token (`get_owned` only).
#[derive(Debug, Clone)]
pub struct BridgeRow {
    pub id: Uuid,
    pub url: String,
    pub token_enc: Vec<u8>,
    pub symbols_json: Option<String>,
    pub last_ok_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

/// A new bridge to insert (`insert`).
#[derive(Debug, Clone)]
pub struct NewBridge {
    pub id: Uuid,
    pub user_id: Uuid,
    pub label: String,
    pub url: String,
    pub token_enc: Vec<u8>,
    pub symbols_json: Option<String>,
    pub last_ok_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// B1.
pub async fn insert(pool: &PgPool, b: &NewBridge) -> Result<(), sqlx::Error> {
    sqlx::query(B1)
        .bind(b.id)
        .bind(b.user_id)
        .bind(&b.label)
        .bind(&b.url)
        .bind(&b.token_enc)
        .bind(&b.symbols_json)
        .bind(b.last_ok_at)
        .bind(&b.last_error)
        .bind(b.created_at)
        .execute(pool)
        .await
        .map(|_| ())
}

/// B2.
pub async fn list_for(pool: &PgPool, user_id: Uuid) -> Result<Vec<BridgeListing>, sqlx::Error> {
    sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(B2)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(
                |(id, label, url, symbols_json, last_ok_at, last_error, created_at)| {
                    BridgeListing {
                        id,
                        label,
                        url,
                        symbols_json,
                        last_ok_at,
                        last_error,
                        created_at,
                    }
                },
            )
            .collect()
    })
}

/// B3. `None` when the bridge does not exist or belongs to another user: the caller answers
/// 404 to both (G5; the admin-gate convention), so the two are indistinguishable.
pub async fn get_owned(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
) -> Result<Option<BridgeRow>, sqlx::Error> {
    sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Vec<u8>,
            Option<String>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<String>,
        ),
    >(B3)
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map(|r| {
        r.map(
            |(id, url, token_enc, symbols_json, last_ok_at, last_error)| BridgeRow {
                id,
                url,
                token_enc,
                symbols_json,
                last_ok_at,
                last_error,
            },
        )
    })
}

/// B4. `Ok(true)` when this user's delete removed the row.
pub async fn delete_owned(pool: &PgPool, user_id: Uuid, id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query(B4)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|r| r.rows_affected() > 0)
}

/// B5: the Test route's writeback (cached symbols, when it last worked, or the exact error).
pub async fn set_test_result(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    symbols_json: Option<&str>,
    ok_at: Option<chrono::DateTime<chrono::Utc>>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(B5)
        .bind(symbols_json)
        .bind(ok_at)
        .bind(error)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sql_shape_violations;

    #[test]
    fn every_store_statement_is_single_table() {
        assert_eq!(STATEMENTS.len(), 5);
        for s in STATEMENTS {
            let v = sql_shape_violations(s);
            assert!(v.is_empty(), "{s}\n  -> {v:?}");
        }
    }
}
