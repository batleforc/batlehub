use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use batlehub_core::{error::CoreError, ports::{ArtifactMeta, ArtifactMetaRepository}};

pub struct PgArtifactMetaRepository {
    pool: PgPool,
}

impl PgArtifactMetaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ArtifactMetaRepository for PgArtifactMetaRepository {
    async fn record_artifact(
        &self,
        key: &str,
        registry: &str,
        package_name: &str,
        version: &str,
        size: Option<u64>,
    ) -> Result<(), CoreError> {
        sqlx::query(
            r#"
            INSERT INTO artifact_cache_meta
                (artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at)
            VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
            ON CONFLICT (artifact_key) DO UPDATE
                SET size_bytes       = EXCLUDED.size_bytes,
                    cached_at        = EXCLUDED.cached_at,
                    last_accessed_at = NOW()
            "#,
        )
        .bind(key)
        .bind(registry)
        .bind(package_name)
        .bind(version)
        .bind(size.map(|s| s as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn touch_artifact(&self, key: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE artifact_cache_meta SET last_accessed_at = NOW() WHERE artifact_key = $1",
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_artifacts(&self, registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        let query = if registry.is_empty() {
            sqlx::query("SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta ORDER BY cached_at DESC")
                .fetch_all(&self.pool)
        } else {
            sqlx::query("SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE registry = $1 ORDER BY cached_at DESC")
                .bind(registry)
                .fetch_all(&self.pool)
        };
        let rows = query.await.map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(row_to_meta).collect())
    }

    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = sqlx::query(
            "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta ORDER BY registry, package_name, cached_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(row_to_meta).collect())
    }

    async fn delete_artifact_meta(&self, key: &str) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM artifact_cache_meta WHERE artifact_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn list_expired_by_ttl(
        &self,
        registry: &str,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = if registry.is_empty() {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE cached_at < $1",
            )
            .bind(older_than)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE registry = $1 AND cached_at < $2",
            )
            .bind(registry)
            .bind(older_than)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(row_to_meta).collect())
    }

    async fn list_idle(
        &self,
        registry: &str,
        idle_since: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = if registry.is_empty() {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE last_accessed_at < $1",
            )
            .bind(idle_since)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE registry = $1 AND last_accessed_at < $2",
            )
            .bind(registry)
            .bind(idle_since)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(row_to_meta).collect())
    }

    async fn total_size_bytes(&self, registry: &str) -> Result<u64, CoreError> {
        let row = if registry.is_empty() {
            sqlx::query("SELECT COALESCE(SUM(size_bytes), 0)::BIGINT AS total FROM artifact_cache_meta")
                .fetch_one(&self.pool)
                .await
        } else {
            sqlx::query("SELECT COALESCE(SUM(size_bytes), 0)::BIGINT AS total FROM artifact_cache_meta WHERE registry = $1")
                .bind(registry)
                .fetch_one(&self.pool)
                .await
        }
        .map_err(|e| CoreError::Database(e.to_string()))?;
        let total: i64 = row.try_get("total").unwrap_or(0);
        Ok(total as u64)
    }

    async fn list_lru(
        &self,
        registry: &str,
        limit: i64,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        let rows = if registry.is_empty() {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta ORDER BY last_accessed_at ASC LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT artifact_key, registry, package_name, version, size_bytes, cached_at, last_accessed_at FROM artifact_cache_meta WHERE registry = $1 ORDER BY last_accessed_at ASC LIMIT $2",
            )
            .bind(registry)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
        }
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(rows.into_iter().map(row_to_meta).collect())
    }
}

fn row_to_meta(row: sqlx::postgres::PgRow) -> ArtifactMeta {
    ArtifactMeta {
        artifact_key: row.get("artifact_key"),
        registry: row.get("registry"),
        package_name: row.get("package_name"),
        version: row.get("version"),
        size_bytes: row.try_get::<i64, _>("size_bytes").ok().map(|s| s as u64),
        cached_at: row.get("cached_at"),
        last_accessed_at: row.get("last_accessed_at"),
    }
}
