//! PostgreSQL-backed rate-limit counter store.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sqlx::PgPool;

use batlehub_core::error::CoreError;
use batlehub_core::ports::RateLimitStore;

/// Rate-limit counter store backed by a PostgreSQL `rate_limit_counters` table.
///
/// Counters survive restarts and are shared across all server instances that
/// point at the same database, making this suitable for multi-instance deployments.
///
/// Each `increment` call uses an atomic `INSERT … ON CONFLICT DO UPDATE … RETURNING count`
/// to guarantee no lost updates under concurrent load. Rows from expired windows are pruned
/// on each write to prevent unbounded table growth.
///
/// Requires the migration in `crates/adapters/migrations/010_rate_limit.sql`.
pub struct PgRateLimitStore {
    pool: PgPool,
}

impl PgRateLimitStore {
    /// Create a new store backed by the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[async_trait]
impl RateLimitStore for PgRateLimitStore {
    async fn increment(&self, key: &str, window_secs: u32) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Database("window_secs must be > 0".into()));
        }
        let now = Self::now_unix();
        let ws = window_secs as i64;
        let window_start = ((now as i64) / ws) * ws;
        let window_reset = (window_start + ws) as u64;

        // Prune all rows for this key except the current window.
        sqlx::query("DELETE FROM rate_limit_counters WHERE key = $1 AND window_start < $2")
            .bind(key)
            .bind(window_start)
            .execute(&self.pool)
            .await
            .map_err(|e| CoreError::Database(format!("rate_limit prune: {e}")))?;

        let count: i64 = sqlx::query_scalar(
            "INSERT INTO rate_limit_counters (key, window_start, count) \
             VALUES ($1, $2, 1) \
             ON CONFLICT (key, window_start) DO UPDATE \
               SET count = rate_limit_counters.count + 1 \
             RETURNING count",
        )
        .bind(key)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("rate_limit increment: {e}")))?;

        Ok((count as u64, window_reset))
    }
}
