use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{CacheEntry, CacheStore};

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

    pub async fn seed_expired(&self, key: &str, metadata: batlehub_core::entities::PackageMetadata) {
        let entry = CacheEntry {
            metadata,
            cached_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        };
        self.inner.write().await.insert(key.to_owned(), InnerEntry { entry });
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
            match chrono::Duration::from_std(ttl) {
                Ok(d) => entry.expires_at = Some(Utc::now() + d),
                Err(e) => tracing::warn!(key, error = %e, "TTL overflows chrono::Duration; entry stored without expiry"),
            }
        }
        self.inner.write().await.insert(key.to_owned(), InnerEntry { entry });
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> Result<(), CoreError> {
        self.inner.write().await.remove(key);
        Ok(())
    }

    async fn get_stale(&self, key: &str) -> Result<Option<CacheEntry>, CoreError> {
        let map = self.inner.read().await;
        Ok(map.get(key).map(|inner| inner.entry.clone()))
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use batlehub_core::entities::{PackageId, PackageMetadata};

    fn dummy_meta() -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new("npm", "test", "1.0.0"),
            published_at: Some(Utc::now()),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        }
    }

    fn entry() -> CacheEntry {
        CacheEntry { metadata: dummy_meta(), cached_at: Utc::now(), expires_at: None }
    }

    fn expired_entry() -> CacheEntry {
        CacheEntry {
            metadata: dummy_meta(),
            cached_at: Utc::now() - chrono::Duration::hours(2),
            expires_at: Some(Utc::now() - chrono::Duration::hours(1)),
        }
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
        store.inner.write().await.insert("k2".to_owned(), InnerEntry { entry: expired_entry() });
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

    #[tokio::test]
    async fn get_stale_returns_expired_entry_that_get_skips() {
        let store = InMemoryCacheStore::new();
        store.inner.write().await.insert("k1".to_owned(), InnerEntry { entry: expired_entry() });
        assert!(store.get("k1").await.unwrap().is_none(), "get should skip expired entry");
        assert!(store.get_stale("k1").await.unwrap().is_some(), "get_stale should return expired entry");
    }

    #[tokio::test]
    async fn get_stale_returns_none_for_missing_key() {
        let store = InMemoryCacheStore::new();
        assert!(store.get_stale("never-set").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn get_stale_returns_non_expired_entry() {
        let store = InMemoryCacheStore::new();
        store.set("k1", entry(), None).await.unwrap();
        assert!(store.get_stale("k1").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn get_stale_returns_none_after_invalidate() {
        let store = InMemoryCacheStore::new();
        store.inner.write().await.insert("k1".to_owned(), InnerEntry { entry: expired_entry() });
        store.invalidate("k1").await.unwrap();
        assert!(store.get_stale("k1").await.unwrap().is_none());
    }
}
