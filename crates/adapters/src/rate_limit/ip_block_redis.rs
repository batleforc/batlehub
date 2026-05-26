//! Redis-backed IP block store (requires the `cache-redis` feature).

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{BlockedIpInfo, IpBlockStore};

/// IP block store backed by Redis.
///
/// Violation counters use `INCR` with `EXPIRE` (same pattern as `RedisRateLimitStore`).
/// Blocks are stored under `batlehub:ipblock:{ip}` as `"{blocked_at}:{unblock_at}:{reason}"`
/// with a TTL set to the ban duration.
pub struct RedisIpBlockStore {
    conn: ConnectionManager,
}

impl RedisIpBlockStore {
    pub async fn new(url: &str) -> Result<Self, CoreError> {
        let client = redis::Client::open(url)
            .map_err(|e| CoreError::Cache(format!("invalid Redis URL for IP block store: {e}")))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis IP block store connection failed: {e}")))?;
        Ok(Self { conn })
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn violation_key(ip: &str, window_start: u64) -> String {
        format!("batlehub:ipviol:{ip}:{window_start}")
    }

    fn block_key(ip: &str) -> String {
        format!("batlehub:ipblock:{ip}")
    }
}

#[async_trait]
impl IpBlockStore for RedisIpBlockStore {
    async fn record_violation(
        &self,
        ip: &str,
        window_secs: u32,
    ) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Cache("window_secs must be > 0".into()));
        }
        let now = Self::now_unix();
        let ws = window_secs as u64;
        let window_start = (now / ws) * ws;
        let window_reset = window_start + ws;

        let key = Self::violation_key(ip, window_start);
        let mut conn = self.conn.clone();

        let count: u64 = conn
            .incr(&key, 1u64)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis INCR ip_violation: {e}")))?;

        if count == 1 {
            conn.expire::<_, ()>(&key, (window_secs + 1) as i64)
                .await
                .map_err(|e| CoreError::Cache(format!("Redis EXPIRE ip_violation: {e}")))?;
        }

        Ok((count, window_reset))
    }

    async fn is_blocked(&self, ip: &str) -> Result<Option<u64>, CoreError> {
        let key = Self::block_key(ip);
        let mut conn = self.conn.clone();
        let val: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis GET ip_block: {e}")))?;
        let Some(s) = val else {
            return Ok(None);
        };
        // Format: "{blocked_at}:{unblock_at}:{reason}"
        let mut parts = s.splitn(3, ':');
        let _blocked_at = parts.next();
        let unblock_at: u64 = parts
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let now = Self::now_unix();
        if unblock_at > now { Ok(Some(unblock_at)) } else { Ok(None) }
    }

    async fn block_ip(&self, ip: &str, unblock_at: u64, reason: &str) -> Result<(), CoreError> {
        let now = Self::now_unix();
        let key = Self::block_key(ip);
        let val = format!("{now}:{unblock_at}:{reason}");
        let ttl = unblock_at.saturating_sub(now) + 1;
        let mut conn = self.conn.clone();
        conn.set_ex::<_, _, ()>(&key, val, ttl)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis SET ip_block: {e}")))?;
        Ok(())
    }

    async fn unblock_ip(&self, ip: &str) -> Result<(), CoreError> {
        let key = Self::block_key(ip);
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(&key)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis DEL ip_block: {e}")))?;
        Ok(())
    }

    async fn list_blocked(&self) -> Result<Vec<BlockedIpInfo>, CoreError> {
        // Redis KEYS for all block keys — admin-only, not on the hot path.
        // KEYS blocks the Redis server briefly; acceptable here since this endpoint
        // is only called by admins, not by regular proxy traffic.
        let pattern = "batlehub:ipblock:*";
        let mut conn = self.conn.clone();
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(pattern)
            .query_async(&mut conn)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis KEYS ip_block: {e}")))?;

        let now = Self::now_unix();
        let mut result = Vec::new();
        for key in keys {
            let val: Option<String> = conn
                .get(&key)
                .await
                .map_err(|e| CoreError::Cache(format!("Redis GET ip_block list: {e}")))?;
            let Some(s) = val else { continue };
            let mut parts = s.splitn(3, ':');
            let blocked_at: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let unblock_at: u64 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            let reason = parts.next().unwrap_or("").to_owned();
            if unblock_at > now {
                let ip = key
                    .strip_prefix("batlehub:ipblock:")
                    .unwrap_or(&key)
                    .to_owned();
                result.push(BlockedIpInfo { ip, blocked_at, unblock_at, reason });
            }
        }
        Ok(result)
    }
}
