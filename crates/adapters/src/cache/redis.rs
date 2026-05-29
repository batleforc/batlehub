use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use batlehub_core::{
    error::CoreError,
    ports::{CacheEntry, CacheStore},
};

/// Redis key for the live entry (subject to TTL-based eviction).
fn live_key(key: &str) -> String {
    format!("batlehub:cache:{key}")
}

/// Redis key for the stale shadow (no TTL; deleted only by `invalidate`).
fn stale_key(key: &str) -> String {
    format!("batlehub:cache:{key}:stale")
}

fn to_cache_err(e: redis::RedisError) -> CoreError {
    CoreError::Cache(e.to_string())
}

fn serialize(entry: &CacheEntry) -> Result<String, CoreError> {
    serde_json::to_string(entry)
        .map_err(|e| CoreError::Cache(format!("serialize cache entry: {e}")))
}

fn deserialize(raw: &str) -> Result<CacheEntry, CoreError> {
    serde_json::from_str(raw).map_err(|e| CoreError::Cache(format!("deserialize cache entry: {e}")))
}

/// `CacheStore` backed by Redis.
///
/// Each entry is stored under two keys:
/// - `batlehub:cache:{key}` — the live entry, evicted by Redis after the TTL expires.
/// - `batlehub:cache:{key}:stale` — a shadow copy with no TTL, used by `get_stale` to
///   serve cached data when the upstream is unavailable even after the live entry expired.
///
/// `invalidate` deletes both keys. The stale shadow is therefore only absent when the key
/// was never written or was explicitly invalidated — matching the `CacheStore` contract.
pub struct RedisCacheStore {
    conn: ConnectionManager,
}

impl RedisCacheStore {
    /// Connect to Redis and return a store backed by an auto-reconnecting connection manager.
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
impl CacheStore for RedisCacheStore {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = conn.get(live_key(key)).await.map_err(to_cache_err)?;
        raw.map(|s| deserialize(&s)).transpose()
    }

    async fn set(
        &self,
        key: &str,
        mut entry: CacheEntry,
        ttl: Option<Duration>,
    ) -> Result<(), CoreError> {
        if let Some(ttl) = ttl {
            match chrono::Duration::from_std(ttl) {
                Ok(d) => entry.expires_at = Some(Utc::now() + d),
                Err(e) => tracing::warn!(
                    key,
                    error = %e,
                    "TTL overflows chrono::Duration; entry stored without expiry"
                ),
            }
        }

        let payload = serialize(&entry)?;
        let mut conn = self.conn.clone();

        match ttl {
            Some(d) => {
                let secs = d.as_secs().max(1);
                conn.set_ex::<_, _, ()>(live_key(key), &payload, secs)
                    .await
                    .map_err(to_cache_err)?;
            }
            None => {
                conn.set::<_, _, ()>(live_key(key), &payload)
                    .await
                    .map_err(to_cache_err)?;
            }
        }

        // Stale shadow: always written without TTL so get_stale can return it even
        // after the live key has expired.
        conn.set::<_, _, ()>(stale_key(key), &payload)
            .await
            .map_err(to_cache_err)?;

        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(&[live_key(key), stale_key(key)])
            .await
            .map_err(to_cache_err)?;
        Ok(())
    }

    async fn get_stale(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = conn.get(stale_key(key)).await.map_err(to_cache_err)?;
        raw.map(|s| deserialize(&s)).transpose()
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests that exercise serialization and key-name helpers without a
    //! running Redis. Integration tests against a real Redis instance live in
    //! `crates/web/tests/`.

    use chrono::Utc;

    use batlehub_core::entities::{PackageId, PackageMetadata};

    use super::*;

    fn dummy_entry() -> CacheEntry {
        CacheEntry {
            metadata: PackageMetadata {
                id: PackageId::new("npm", "lodash", "4.17.21"),
                published_at: Some(Utc::now()),
                download_url: None,
                checksum: None,
                is_signed: None,
                extra: serde_json::json!({}),
                cache_control: None,
            },
            cached_at: Utc::now(),
            expires_at: None,
        }
    }

    #[test]
    fn round_trip_serialization() {
        let entry = dummy_entry();
        let raw = serialize(&entry).unwrap();
        let decoded = deserialize(&raw).unwrap();
        assert_eq!(decoded.metadata.id.name, "lodash");
        assert_eq!(decoded.metadata.id.version, "4.17.21");
    }

    #[test]
    fn round_trip_with_expiry() {
        let mut entry = dummy_entry();
        entry.expires_at = Some(Utc::now() + chrono::Duration::hours(1));
        let raw = serialize(&entry).unwrap();
        let decoded = deserialize(&raw).unwrap();
        assert!(decoded.expires_at.is_some());
        assert!(!decoded.is_expired());
    }

    #[test]
    fn key_helpers_are_namespaced() {
        assert_eq!(live_key("foo"), "batlehub:cache:foo");
        assert_eq!(stale_key("foo"), "batlehub:cache:foo:stale");
    }

    #[test]
    fn key_helpers_are_distinct() {
        assert_ne!(live_key("k"), stale_key("k"));
    }
}
