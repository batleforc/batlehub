use crate::db::DbResultExt;
use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::{
    error::CoreError,
    ports::{QuotaOutcome, QuotaRepository, QuotaUsage},
};

pub struct PgQuotaRepository {
    pool: PgPool,
}

impl PgQuotaRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl QuotaRepository for PgQuotaRepository {
    async fn get_usage(&self, user_id: &str, registry: &str) -> Result<QuotaUsage, CoreError> {
        let row = sqlx::query(
            "SELECT bytes_published, packages_count \
             FROM quota_usage \
             WHERE user_id = $1 AND registry = $2",
        )
        .bind(user_id)
        .bind(registry)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        Ok(match row {
            Some(r) => {
                let bytes: i64 = r.try_get("bytes_published").unwrap_or(0);
                let count: i32 = r.try_get("packages_count").unwrap_or(0);
                QuotaUsage {
                    user_id: user_id.to_owned(),
                    registry: registry.to_owned(),
                    bytes_published: bytes.max(0) as u64,
                    packages_count: count.max(0) as u32,
                }
            }
            None => QuotaUsage {
                user_id: user_id.to_owned(),
                registry: registry.to_owned(),
                bytes_published: 0,
                packages_count: 0,
            },
        })
    }

    async fn record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO quota_usage (user_id, registry, bytes_published, packages_count, updated_at) \
             VALUES ($1, $2, $3, 1, NOW()) \
             ON CONFLICT (user_id, registry) DO UPDATE SET \
                 bytes_published = quota_usage.bytes_published + $3, \
                 packages_count  = quota_usage.packages_count + 1, \
                 updated_at      = NOW()",
        )
        .bind(user_id)
        .bind(registry)
        .bind(bytes as i64)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn try_record_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
        max_bytes: Option<u64>,
        max_packages: Option<u32>,
    ) -> Result<QuotaOutcome, CoreError> {
        let mut tx = self.pool.begin().await.db_err()?;

        // Ensure the row exists, then lock it for the duration of the
        // transaction so a concurrent `try_record_publish` for the same
        // (user, registry) blocks until this one commits or rolls back —
        // that's what makes the check-then-write atomic.
        sqlx::query(
            "INSERT INTO quota_usage (user_id, registry, bytes_published, packages_count, updated_at) \
             VALUES ($1, $2, 0, 0, NOW()) \
             ON CONFLICT (user_id, registry) DO NOTHING",
        )
        .bind(user_id)
        .bind(registry)
        .execute(&mut *tx)
        .await
        .db_err()?;

        let row = sqlx::query(
            "SELECT bytes_published, packages_count FROM quota_usage \
             WHERE user_id = $1 AND registry = $2 FOR UPDATE",
        )
        .bind(user_id)
        .bind(registry)
        .fetch_one(&mut *tx)
        .await
        .db_err()?;

        let current_bytes: i64 = row.try_get("bytes_published").unwrap_or(0);
        let current_packages: i32 = row.try_get("packages_count").unwrap_or(0);
        let new_bytes = current_bytes.max(0) as u64 + bytes;
        let new_packages = current_packages.max(0) as u32 + 1;

        let exceeded = max_bytes.is_some_and(|max| new_bytes > max)
            || max_packages.is_some_and(|max| new_packages > max);

        if exceeded {
            tx.rollback().await.db_err()?;
            return Ok(QuotaOutcome::Exceeded {
                bytes_used: new_bytes,
                packages_used: new_packages,
            });
        }

        sqlx::query(
            "UPDATE quota_usage SET bytes_published = $3, packages_count = $4, updated_at = NOW() \
             WHERE user_id = $1 AND registry = $2",
        )
        .bind(user_id)
        .bind(registry)
        .bind(new_bytes as i64)
        .bind(new_packages as i32)
        .execute(&mut *tx)
        .await
        .db_err()?;

        tx.commit().await.db_err()?;

        Ok(QuotaOutcome::Recorded {
            bytes_used: new_bytes,
            packages_used: new_packages,
        })
    }

    async fn revoke_publish(
        &self,
        user_id: &str,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE quota_usage SET \
                 bytes_published = GREATEST(0, bytes_published - $3), \
                 packages_count  = GREATEST(0, packages_count - 1), \
                 updated_at      = NOW() \
             WHERE user_id = $1 AND registry = $2",
        )
        .bind(user_id)
        .bind(registry)
        .bind(bytes as i64)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn reset_usage(&self, user_id: &str, registry: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE quota_usage SET \
                 bytes_published = 0, \
                 packages_count  = 0, \
                 updated_at      = NOW() \
             WHERE user_id = $1 AND registry = $2",
        )
        .bind(user_id)
        .bind(registry)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn list_usage(&self, registry: Option<&str>) -> Result<Vec<QuotaUsage>, CoreError> {
        let rows: Vec<QuotaUsage> = if let Some(reg) = registry {
            sqlx::query(
                "SELECT user_id, registry, bytes_published, packages_count \
                 FROM quota_usage \
                 WHERE registry = $1 \
                 ORDER BY bytes_published DESC",
            )
            .bind(reg)
            .fetch_all(&self.pool)
            .await
            .db_err()?
            .into_iter()
            .map(|r| {
                let bytes: i64 = r.try_get("bytes_published").unwrap_or(0);
                let count: i32 = r.try_get("packages_count").unwrap_or(0);
                QuotaUsage {
                    user_id: r.try_get("user_id").unwrap_or_default(),
                    registry: r.try_get("registry").unwrap_or_default(),
                    bytes_published: bytes.max(0) as u64,
                    packages_count: count.max(0) as u32,
                }
            })
            .collect()
        } else {
            sqlx::query(
                "SELECT user_id, registry, bytes_published, packages_count \
                 FROM quota_usage \
                 ORDER BY registry, bytes_published DESC",
            )
            .fetch_all(&self.pool)
            .await
            .db_err()?
            .into_iter()
            .map(|r| {
                let bytes: i64 = r.try_get("bytes_published").unwrap_or(0);
                let count: i32 = r.try_get("packages_count").unwrap_or(0);
                QuotaUsage {
                    user_id: r.try_get("user_id").unwrap_or_default(),
                    registry: r.try_get("registry").unwrap_or_default(),
                    bytes_published: bytes.max(0) as u64,
                    packages_count: count.max(0) as u32,
                }
            })
            .collect()
        };
        Ok(rows)
    }
}
