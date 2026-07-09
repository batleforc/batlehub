use std::time::Duration;

use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use batlehub_core::error::CoreError;
use batlehub_core::ports::WarmCoordinator;

fn claim_key(key: &str) -> String {
    format!("batlehub:warm:{key}")
}

fn to_err(e: redis::RedisError) -> CoreError {
    CoreError::Cache(e.to_string())
}

/// Redis-backed `WarmCoordinator`.
///
/// Uses `SET … NX PX` to claim warming rights for a single artifact.
/// The first replica to call `try_claim` with a given key wins; others
/// receive `false` and skip the download. The claim expires automatically
/// after `ttl` so a crashed replica cannot block future warm-up runs.
pub struct RedisWarmCoordinator {
    conn: ConnectionManager,
}

impl RedisWarmCoordinator {
    pub async fn new(url: &str) -> Result<Self, CoreError> {
        let client = redis::Client::open(url)
            .map_err(|e| CoreError::Cache(format!("invalid Redis URL: {e}")))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| CoreError::Cache(format!("Redis connection failed: {e}")))?;
        Ok(Self { conn })
    }
}

#[async_trait]
impl WarmCoordinator for RedisWarmCoordinator {
    async fn try_claim(&self, key: &str, ttl: Duration) -> bool {
        let mut conn = self.conn.clone();
        let ttl_ms = ttl.as_millis().min(u64::MAX as u128) as u64;
        let result: Result<Option<String>, _> = conn
            .set_options(
                claim_key(key),
                "1",
                redis::SetOptions::default()
                    .conditional_set(redis::ExistenceCheck::NX)
                    .get(false)
                    .with_expiration(redis::SetExpiry::PX(ttl_ms)),
            )
            .await;
        match result {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                tracing::warn!(key, error = %to_err(e), "warm_coordinator: try_claim failed; skipping");
                false
            }
        }
    }

    async fn release(&self, key: &str) {
        let mut conn = self.conn.clone();
        if let Err(e) = conn.del::<_, ()>(claim_key(key)).await {
            tracing::warn!(key, error = %to_err(e), "warm_coordinator: release failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_err_wraps_redis_error_as_cache_error() {
        let redis_err = redis::RedisError::from((redis::ErrorKind::Io, "boom"));
        match to_err(redis_err) {
            CoreError::Cache(msg) => assert!(msg.contains("boom")),
            other => panic!("expected CoreError::Cache, got {other:?}"),
        }
    }

    #[test]
    fn claim_key_is_namespaced() {
        assert_eq!(claim_key("npm/foo"), "batlehub:warm:npm/foo");
    }
}
