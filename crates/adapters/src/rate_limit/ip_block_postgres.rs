//! PostgreSQL-backed IP block store.

use async_trait::async_trait;
use sqlx::PgPool;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{BlockedIpInfo, IpBlockStore};

use super::now_unix;

/// IP block store backed by the `ip_violation_counters` and `ip_blocks` tables.
///
/// Blocks and violation counts survive restarts and are shared across all
/// server instances that point at the same database.
///
/// Requires the migration in `crates/adapters/migrations/014_ip_blocks.sql`.
pub struct PgIpBlockStore {
    pool: PgPool,
}

impl PgIpBlockStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl IpBlockStore for PgIpBlockStore {
    async fn record_violation(
        &self,
        ip: &str,
        window_secs: u32,
    ) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Database("window_secs must be > 0".into()));
        }
        let now = now_unix();
        let ws = window_secs as i64;
        let window_start = ((now as i64) / ws) * ws;
        let window_reset = (window_start + ws) as u64;

        // Prune expired windows for this IP.
        sqlx::query(
            "DELETE FROM ip_violation_counters WHERE ip = $1 AND window_start < $2",
        )
        .bind(ip)
        .bind(window_start)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("ip_block prune: {e}")))?;

        let count: i64 = sqlx::query_scalar(
            "INSERT INTO ip_violation_counters (ip, window_start, count) \
             VALUES ($1, $2, 1) \
             ON CONFLICT (ip, window_start) DO UPDATE \
               SET count = ip_violation_counters.count + 1 \
             RETURNING count",
        )
        .bind(ip)
        .bind(window_start)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("ip_block increment: {e}")))?;

        Ok((count as u64, window_reset))
    }

    async fn is_blocked(&self, ip: &str) -> Result<Option<u64>, CoreError> {
        let now = now_unix() as i64;
        let row: Option<i64> = sqlx::query_scalar(
            "SELECT unblock_at FROM ip_blocks WHERE ip = $1 AND unblock_at > $2",
        )
        .bind(ip)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("ip_block is_blocked: {e}")))?;
        Ok(row.map(|v| v as u64))
    }

    async fn block_ip(&self, ip: &str, unblock_at: u64, reason: &str) -> Result<(), CoreError> {
        let now = now_unix() as i64;
        sqlx::query(
            "INSERT INTO ip_blocks (ip, blocked_at, unblock_at, reason) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (ip) DO UPDATE \
               SET blocked_at = EXCLUDED.blocked_at, \
                   unblock_at = EXCLUDED.unblock_at, \
                   reason     = EXCLUDED.reason",
        )
        .bind(ip)
        .bind(now)
        .bind(unblock_at as i64)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("ip_block block_ip: {e}")))?;
        Ok(())
    }

    async fn unblock_ip(&self, ip: &str) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM ip_blocks WHERE ip = $1")
            .bind(ip)
            .execute(&self.pool)
            .await
            .map_err(|e| CoreError::Database(format!("ip_block unblock_ip: {e}")))?;
        Ok(())
    }

    async fn list_blocked(&self) -> Result<Vec<BlockedIpInfo>, CoreError> {
        let now = now_unix() as i64;
        let rows = sqlx::query(
            "SELECT ip, blocked_at, unblock_at, reason \
             FROM ip_blocks \
             WHERE unblock_at > $1 \
             ORDER BY blocked_at DESC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(format!("ip_block list_blocked: {e}")))?;

        use sqlx::Row;
        Ok(rows
            .into_iter()
            .map(|r| BlockedIpInfo {
                ip: r.get("ip"),
                blocked_at: r.get::<i64, _>("blocked_at") as u64,
                unblock_at: r.get::<i64, _>("unblock_at") as u64,
                reason: r.get("reason"),
            })
            .collect())
    }
}
