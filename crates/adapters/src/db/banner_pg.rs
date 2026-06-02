use async_trait::async_trait;
use sqlx::PgPool;
use crate::db::DbResultExt;

use batlehub_core::{entities::GlobalBanner, error::CoreError, ports::BannerPort};

const BANNER_KEY: &str = "banner";

/// PostgreSQL-backed banner store using the `system_kv` table.
/// Suitable for multi-instance (HA) deployments.
pub struct PgBannerStore {
    pool: PgPool,
}

impl PgBannerStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BannerPort for PgBannerStore {
    async fn get(&self) -> Result<Option<GlobalBanner>, CoreError> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT value FROM system_kv WHERE key = $1")
                .bind(BANNER_KEY)
                .fetch_optional(&self.pool)
                .await
                .db_err()?;
        match row {
            None => Ok(None),
            Some((val,)) => {
                let banner: GlobalBanner = serde_json::from_value(val)
                    .map_err(|e| CoreError::Database(format!("banner deserialize: {e}")))?;
                Ok(Some(banner))
            }
        }
    }

    async fn set(&self, banner: GlobalBanner) -> Result<(), CoreError> {
        let val = serde_json::to_value(&banner)
            .map_err(|e| CoreError::Database(format!("banner serialize: {e}")))?;
        sqlx::query(
            "INSERT INTO system_kv (key, value, updated_at) VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = NOW()",
        )
        .bind(BANNER_KEY)
        .bind(val)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn clear(&self) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM system_kv WHERE key = $1")
            .bind(BANNER_KEY)
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(())
    }
}
