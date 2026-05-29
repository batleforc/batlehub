use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};

use batlehub_core::{
    entities::PackageMetadata,
    error::CoreError,
    ports::{CacheEntry, CacheStore},
};

pub struct PgCacheStore {
    pool: PgPool,
}

impl PgCacheStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CacheStore for PgCacheStore {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let row = sqlx::query(
            "SELECT metadata, cached_at, expires_at FROM metadata_cache \
             WHERE cache_key = $1 AND (expires_at IS NULL OR expires_at > NOW())",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        row.map(|r| decode_row(&r)).transpose()
    }

    async fn set(
        &self,
        key: &str,
        entry: CacheEntry,
        ttl: Option<Duration>,
    ) -> Result<(), CoreError> {
        let expires_at = ttl.and_then(|d| match chrono::Duration::from_std(d) {
            Ok(cd) => Some(Utc::now() + cd),
            Err(e) => {
                tracing::warn!(key, error = %e, "TTL overflows chrono::Duration; entry stored without expiry");
                None
            }
        });
        let metadata_json = serde_json::to_value(&entry.metadata)
            .map_err(|e| CoreError::Database(format!("serialize cache metadata: {e}")))?;

        sqlx::query(
            "INSERT INTO metadata_cache (cache_key, metadata, cached_at, expires_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (cache_key) DO UPDATE SET \
                 metadata   = EXCLUDED.metadata, \
                 cached_at  = EXCLUDED.cached_at, \
                 expires_at = EXCLUDED.expires_at",
        )
        .bind(key)
        .bind(metadata_json)
        .bind(entry.cached_at)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM metadata_cache WHERE cache_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_stale(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let row = sqlx::query(
            "SELECT metadata, cached_at, expires_at FROM metadata_cache WHERE cache_key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        row.map(|r| decode_row(&r)).transpose()
    }
}

fn decode_row(r: &sqlx::postgres::PgRow) -> Result<CacheEntry, CoreError> {
    let metadata_json: serde_json::Value = r.get("metadata");
    let metadata: PackageMetadata = serde_json::from_value(metadata_json)
        .map_err(|e| CoreError::Database(format!("deserialize cache metadata: {e}")))?;
    Ok(CacheEntry {
        metadata,
        cached_at: r.get("cached_at"),
        expires_at: r.get("expires_at"),
    })
}
