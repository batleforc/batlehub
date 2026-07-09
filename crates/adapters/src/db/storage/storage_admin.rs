use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::error::CoreError;
use batlehub_core::ports::{ArtifactStorageRecord, StorageAdminRepository};

use crate::db::DbResultExt;

/// PostgreSQL-backed admin access to the `artifact_storage` tracking table:
/// per-key backend lookups and prefix-based bulk deletes for the back-office
/// cache-clear and package-detail endpoints.
pub struct PgStorageAdminRepository {
    pool: PgPool,
}

impl PgStorageAdminRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl StorageAdminRepository for PgStorageAdminRepository {
    async fn find_by_key(
        &self,
        storage_key: &str,
    ) -> Result<Option<ArtifactStorageRecord>, CoreError> {
        let row = sqlx::query(
            "SELECT backend_name, stored_at FROM artifact_storage WHERE storage_key = $1",
        )
        .bind(storage_key)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;
        Ok(row.map(|r| ArtifactStorageRecord {
            backend_name: r.get("backend_name"),
            stored_at: r.get("stored_at"),
        }))
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<u64, CoreError> {
        let result = sqlx::query("DELETE FROM artifact_storage WHERE storage_key LIKE $1")
            .bind(format!("{prefix}%"))
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(result.rows_affected())
    }
}
