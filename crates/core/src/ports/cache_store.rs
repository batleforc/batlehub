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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;
    use crate::entities::{PackageId, PackageMetadata};

    fn dummy_meta() -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new("npm", "test", "1.0.0"),
            published_at: Some(Utc::now()),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
        }
    }

    fn entry() -> CacheEntry {
        CacheEntry { metadata: dummy_meta(), cached_at: Utc::now(), expires_at: None }
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_key() {
        let store = InMemoryCacheStore::new();
        assert!(store.get("missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_and_get_returns_entry() {
        let store = InMemoryCacheStore::new();
        store.set("k1", entry(), None).await.unwrap();
        let got = store.get("k1").await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().metadata.id.name, "test");
    }

    #[tokio::test]
    async fn invalidate_removes_entry() {
        let store = InMemoryCacheStore::new();
        store.set("k1", entry(), None).await.unwrap();
        store.invalidate("k1").await.unwrap();
        assert!(store.get("k1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn expired_entry_treated_as_miss() {
        let store = InMemoryCacheStore::new();
        // Set with a 1-nanosecond TTL so it expires immediately
        store.set("k1", entry(), Some(Duration::from_nanos(1))).await.unwrap();
        // Sleep is unreliable in tests; instead set expires_at directly via a past timestamp
        // by manipulating the entry after insertion
        let mut e = entry();
        e.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        // Re-insert with already-expired timestamp
        store.inner.write().await.insert("k2".to_owned(), super::InnerEntry { entry: e });
        assert!(store.get("k2").await.unwrap().is_none(), "expired entry should be treated as a cache miss");
    }

    #[tokio::test]
    async fn set_overwrites_existing_entry() {
        let store = InMemoryCacheStore::new();
        store.set("k1", entry(), None).await.unwrap();

        let mut e2 = entry();
        e2.metadata.id = PackageId::new("cargo", "serde", "2.0.0");
        store.set("k1", e2, None).await.unwrap();

        let got = store.get("k1").await.unwrap().unwrap();
        assert_eq!(got.metadata.id.registry, "cargo");
    }
}
