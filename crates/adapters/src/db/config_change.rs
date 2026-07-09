use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::error::CoreError;
use batlehub_core::ports::{ConfigChangeRecord, ConfigChangeRepository};

use super::DbResultExt;

/// PostgreSQL-backed audit trail for hot-reload config changes.
///
/// Backs the `config_changes` table used by `ConfigReloadService` to record
/// every applied (or failed) reload attempt.
pub struct PgConfigChangeRepository {
    pool: PgPool,
}

impl PgConfigChangeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ConfigChangeRepository for PgConfigChangeRepository {
    async fn insert(&self, record: ConfigChangeRecord) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO config_changes (id, triggered_by, triggered_at, status, diff, summary, error_msg)
             VALUES ($1, $2, NOW(), $3, $4, $5, $6)",
        )
        .bind(record.id)
        .bind(&record.triggered_by)
        .bind(&record.status)
        .bind(&record.diff)
        .bind(&record.summary)
        .bind(&record.error_msg)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn list(&self, page: u64, per_page: u64) -> Result<Vec<ConfigChangeRecord>, CoreError> {
        let offset = (page * per_page) as i64;
        let limit = per_page as i64;
        let rows = sqlx::query(
            "SELECT id, triggered_by, triggered_at, status, diff, summary, error_msg
             FROM config_changes
             ORDER BY triggered_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows
            .into_iter()
            .map(|r| ConfigChangeRecord {
                id: r.get("id"),
                triggered_by: r.get("triggered_by"),
                triggered_at: r.get("triggered_at"),
                status: r.get("status"),
                diff: r
                    .try_get::<serde_json::Value, _>("diff")
                    .unwrap_or(serde_json::Value::Object(Default::default())),
                summary: r.get("summary"),
                error_msg: r.try_get("error_msg").ok(),
            })
            .collect())
    }

    async fn count(&self) -> Result<u64, CoreError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM config_changes")
            .fetch_one(&self.pool)
            .await
            .db_err()?;
        Ok(count as u64)
    }
}
