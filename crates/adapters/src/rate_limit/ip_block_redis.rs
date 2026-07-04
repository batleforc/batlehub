//! Redis-backed IP block store (requires the `cache-redis` feature).

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{BlockedIpInfo, IpBlockStore};

use super::now_unix;

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
        let conn = ConnectionManager::new(client).await.map_err(|e| {
            CoreError::Cache(format!("Redis IP block store connection failed: {e}"))
        })?;
        Ok(Self { conn })
    }

    fn violation_key(ip: &str, window_start: u64) -> String {
        format!("batlehub:ipviol:{ip}:{window_start}")
    }

    fn block_key(ip: &str) -> String {
        format!("batlehub:ipblock:{ip}")
    }
}

/// Parse the stored block value `"{blocked_at}:{unblock_at}:{reason}"`.
/// Returns `(blocked_at, unblock_at, reason)` or zeros/empty on malformed input.
fn parse_block_value(s: &str) -> (u64, u64, &str) {
    let mut parts = s.splitn(3, ':');
    let blocked_at = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let unblock_at = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let reason = parts.next().unwrap_or("");
    (blocked_at, unblock_at, reason)
}

#[async_trait]
impl IpBlockStore for RedisIpBlockStore {
    async fn record_violation(&self, ip: &str, window_secs: u32) -> Result<(u64, u64), CoreError> {
        if window_secs == 0 {
            return Err(CoreError::Cache("window_secs must be > 0".into()));
        }
        let now = now_unix();
        let (window_start, window_reset) = super::violation_window(now, window_secs);

        let key = Self::violation_key(ip, window_start);
        let mut conn = self.conn.clone();

        let count: u64 = conn
            .incr(&key, 1u64)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis INCR ip_violation: {e}")))?;

        // Set TTL on the first write. Logged but not fatal: a missed EXPIRE means
        // the key won't auto-delete, but it will be ignored once the window changes.
        if count == 1 {
            if let Err(e) = conn.expire::<_, ()>(&key, (window_secs + 1) as i64).await {
                tracing::warn!(error = %e, %key, "failed to set TTL on violation counter; key may not expire");
            }
        }

        Ok((count, window_reset))
    }

    async fn blocked_until(&self, ip: &str) -> Result<Option<u64>, CoreError> {
        let key = Self::block_key(ip);
        let mut conn = self.conn.clone();
        let val: Option<String> = conn
            .get(&key)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis GET ip_block: {e}")))?;
        let Some(s) = val else {
            return Ok(None);
        };
        let (_, unblock_at, _) = parse_block_value(&s);
        let now = now_unix();
        if unblock_at > now {
            Ok(Some(unblock_at))
        } else {
            Ok(None)
        }
    }

    async fn block_ip(&self, ip: &str, unblock_at: u64, reason: &str) -> Result<(), CoreError> {
        let now = now_unix();
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
        let mut conn = self.conn.clone();

        // SCAN iterates non-blocking (unlike KEYS which stalls Redis for the full scan).
        let mut cursor: u64 = 0;
        let mut keys: Vec<String> = Vec::new();
        loop {
            let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("batlehub:ipblock:*")
                .arg("COUNT")
                .arg(100u64)
                .query_async(&mut conn)
                .await
                .map_err(|e| CoreError::Cache(format!("Redis SCAN ip_block: {e}")))?;
            keys.extend(batch);
            cursor = next_cursor;
            if cursor == 0 {
                break;
            }
        }

        if keys.is_empty() {
            return Ok(vec![]);
        }

        // Fetch all values in one MGET round-trip.
        let values: Vec<Option<String>> = redis::cmd("MGET")
            .arg(&keys)
            .query_async(&mut conn)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis MGET ip_block: {e}")))?;

        let now = now_unix();
        let result = keys
            .into_iter()
            .zip(values)
            .filter_map(|(key, val)| {
                let s = val?;
                let (blocked_at, unblock_at, reason) = parse_block_value(&s);
                if unblock_at <= now {
                    return None;
                }
                let ip = key
                    .strip_prefix("batlehub:ipblock:")
                    .unwrap_or(&key)
                    .to_owned();
                Some(BlockedIpInfo {
                    ip,
                    blocked_at,
                    unblock_at,
                    reason: reason.to_owned(),
                })
            })
            .collect();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn violation_key_format() {
        assert_eq!(
            RedisIpBlockStore::violation_key("1.2.3.4", 1_704_067_200),
            "batlehub:ipviol:1.2.3.4:1704067200"
        );
    }

    #[test]
    fn block_key_format() {
        assert_eq!(
            RedisIpBlockStore::block_key("10.0.0.1"),
            "batlehub:ipblock:10.0.0.1"
        );
    }

    #[test]
    fn parse_block_value_valid() {
        let (blocked_at, unblock_at, reason) = parse_block_value("1000:2000:rate-limit");
        assert_eq!(blocked_at, 1000);
        assert_eq!(unblock_at, 2000);
        assert_eq!(reason, "rate-limit");
    }

    #[test]
    fn parse_block_value_reason_with_colon() {
        let (_, _, reason) = parse_block_value("1000:2000:too many:requests");
        assert_eq!(reason, "too many:requests");
    }

    #[test]
    fn parse_block_value_malformed() {
        let (blocked_at, unblock_at, reason) = parse_block_value("bad");
        assert_eq!(blocked_at, 0);
        assert_eq!(unblock_at, 0);
        assert_eq!(reason, "");
    }

    #[test]
    fn violation_window_aligns_to_boundary() {
        // super = ip_block_redis module; super::super = rate_limit module (where violation_window lives)
        let (start, reset) = super::super::violation_window(1000, 60);
        assert_eq!(start, 960);
        assert_eq!(reset, 1020);
        assert!(reset > 1000);
    }
}
