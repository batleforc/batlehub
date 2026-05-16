use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

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
}

// ── In-memory implementation (default, no external deps) ──────────────────────

#[derive(Debug)]
struct InnerEntry {
    entry: CacheEntry,
}

#[derive(Debug, Default)]
pub struct InMemoryCacheStore {
    inner: Arc<RwLock<HashMap<String, InnerEntry>>>,
}

impl InMemoryCacheStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl CacheStore for InMemoryCacheStore {
    async fn get(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let map = self.inner.read().await;
        match map.get(key) {
            Some(inner) if !inner.entry.is_expired() => Ok(Some(inner.entry.clone())),
            Some(_) => Ok(None), // expired, treat as miss
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, mut entry: CacheEntry, ttl: Option<Duration>) -> Result<(), CoreError> {
        if let Some(ttl) = ttl {
            let exp = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
            entry.expires_at = Some(exp);
        }
        self.inner.write().await.insert(key.to_owned(), InnerEntry { entry });
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        self.inner.write().await.remove(key);
        Ok(())
    }
}
