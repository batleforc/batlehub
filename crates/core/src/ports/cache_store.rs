use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::entities::PackageMetadata;
use crate::error::CoreError;

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub metadata: PackageMetadata,
    pub cached_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CacheEntry {
    pub fn is_expired(&self) -> bool {
        match self.expires_at {
            Some(exp) => Utc::now() > exp,
            None => false,
        }
    }
}

/// Metadata cache to avoid hitting upstream on every request.
///
/// Only caches `PackageMetadata` (version info, published_at, download URLs).
/// Actual artifact bytes are managed by `StorageBackend`.
#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CoreError>;

    async fn set(&self, key: &str, entry: CacheEntry, ttl: Option<Duration>) -> Result<(), CoreError>;

    async fn invalidate(&self, key: &str) -> Result<(), CoreError>;

    /// Returns a cached entry regardless of whether it has expired.
    /// Returns `None` only if the key has never been set or was explicitly invalidated.
    /// Used for stale-while-unavailable fallback when upstream is down.
    async fn get_stale(&self, key: &str) -> Result<Option<CacheEntry>, CoreError>;
}
