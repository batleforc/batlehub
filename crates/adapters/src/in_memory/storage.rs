use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream;
use tokio::sync::RwLock;

use batlehub_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoredArtifact},
};

/// In-memory [`StorageBackend`].
///
/// Stores artifact bytes and [`StorageMeta`] in a `RwLock`-protected hash map.
/// All keys, including those with colons or slashes, are accepted as-is.
#[derive(Debug, Default)]
pub struct InMemoryStorageBackend {
    data: Arc<RwLock<HashMap<String, (Bytes, StorageMeta)>>>,
}

impl InMemoryStorageBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl StorageBackend for InMemoryStorageBackend {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        self.data.write().await.insert(key.to_owned(), (data, meta));
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let map = self.data.read().await;
        Ok(map.get(key).map(|(data, meta)| {
            let bytes = data.clone();
            let s: ByteStream =
                Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(bytes) }));
            StoredArtifact {
                stream: s,
                meta: meta.clone(),
            }
        }))
    }

    /// Atomic key move under a single write lock. `store_streaming` falls back to
    /// the trait default (collect + `store`), which is fine for an in-RAM map.
    async fn move_key(&self, from: &str, to: &str) -> Result<(), CoreError> {
        let mut map = self.data.write().await;
        match map.remove(from) {
            Some(entry) => {
                map.insert(to.to_owned(), entry);
                Ok(())
            }
            None => Err(CoreError::Storage(format!(
                "move_key source '{from}' does not exist"
            ))),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.read().await.contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.write().await.remove(key).is_some())
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let mut map = self.data.write().await;
        let keys: Vec<String> = map
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let count = keys.len();
        for k in keys {
            map.remove(&k);
        }
        Ok(count)
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let map = self.data.read().await;
        Ok(map.iter().filter(|(k, _)| k.starts_with(prefix)).fold(
            (0u64, 0u64),
            |(count, bytes), (_, (data, meta))| {
                (count + 1, bytes + meta.size.unwrap_or(data.len() as u64))
            },
        ))
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        Ok(self
            .data
            .read()
            .await
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::StreamExt;

    use batlehub_core::error::CoreError;
    use batlehub_core::ports::{StorageBackend, StorageMeta};

    use super::InMemoryStorageBackend;

    fn meta(size: u64) -> StorageMeta {
        StorageMeta {
            size: Some(size),
            content_type: None,
            checksum: None,
        }
    }

    #[tokio::test]
    async fn store_then_retrieve_round_trips() {
        let s = InMemoryStorageBackend::new();
        s.store("k", Bytes::from("hello"), meta(5)).await.unwrap();
        let artifact = s.retrieve("k").await.unwrap().expect("should exist");
        assert_eq!(artifact.meta.size, Some(5));
    }

    #[tokio::test]
    async fn retrieve_missing_returns_none() {
        let s = InMemoryStorageBackend::new();
        assert!(s.retrieve("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn exists_before_and_after_store() {
        let s = InMemoryStorageBackend::new();
        assert!(!s.exists("k").await.unwrap());
        s.store("k", Bytes::from("x"), meta(1)).await.unwrap();
        assert!(s.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_key() {
        let s = InMemoryStorageBackend::new();
        s.store("k", Bytes::from("x"), meta(1)).await.unwrap();
        s.delete("k").await.unwrap();
        assert!(!s.exists("k").await.unwrap());
    }

    #[tokio::test]
    async fn move_key_relocates_entry() {
        let s = InMemoryStorageBackend::new();
        s.store("from", Bytes::from("payload"), meta(7))
            .await
            .unwrap();
        s.move_key("from", "to").await.unwrap();
        assert!(!s.exists("from").await.unwrap());
        let artifact = s.retrieve("to").await.unwrap().expect("should exist");
        assert_eq!(artifact.meta.size, Some(7));
    }

    #[tokio::test]
    async fn move_key_missing_source_errors() {
        let s = InMemoryStorageBackend::new();
        assert!(s.move_key("ghost", "dest").await.is_err());
    }

    #[tokio::test]
    async fn store_streaming_default_collects_and_hashes() {
        use futures::stream;
        let s = InMemoryStorageBackend::new();
        let chunks: Vec<Result<Bytes, CoreError>> = vec![
            Ok(Bytes::from_static(b"he")),
            Ok(Bytes::from_static(b"llo")),
        ];
        let outcome = s
            .store_streaming("k", Box::pin(stream::iter(chunks)), StorageMeta::default())
            .await
            .unwrap();
        assert_eq!(
            outcome.content_hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(outcome.size, 5);
        let artifact = s.retrieve("k").await.unwrap().unwrap();
        let mut stream = artifact.stream;
        let mut buf = Vec::new();
        while let Some(c) = stream.next().await {
            buf.extend_from_slice(&c.unwrap());
        }
        assert_eq!(buf, b"hello");
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_matching_only() {
        let s = InMemoryStorageBackend::new();
        for key in ["p/a", "p/b", "q/c"] {
            s.store(key, Bytes::from("x"), meta(1)).await.unwrap();
        }
        assert_eq!(s.delete_by_prefix("p/").await.unwrap(), 2);
        assert!(!s.exists("p/a").await.unwrap());
        assert!(s.exists("q/c").await.unwrap());
    }

    #[tokio::test]
    async fn stat_by_prefix_sums_sizes() {
        let s = InMemoryStorageBackend::new();
        s.store("p/a", Bytes::from("abc"), meta(3)).await.unwrap();
        s.store("p/b", Bytes::from("de"), meta(2)).await.unwrap();
        s.store("q/c", Bytes::from("f"), meta(1)).await.unwrap();
        let (count, bytes) = s.stat_by_prefix("p/").await.unwrap();
        assert_eq!((count, bytes), (2, 5));
    }

    #[tokio::test]
    async fn list_keys_returns_prefix_matches() {
        let s = InMemoryStorageBackend::new();
        for key in ["ns/x", "ns/y", "other/z"] {
            s.store(key, Bytes::from("v"), meta(1)).await.unwrap();
        }
        let mut keys = s.list_keys("ns/").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["ns/x", "ns/y"]);
    }
}
