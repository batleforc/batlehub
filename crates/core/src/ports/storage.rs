use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;

use crate::error::CoreError;

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, CoreError>> + Send + 'static>>;

#[derive(Debug, Clone, Default)]
pub struct StorageMeta {
    pub content_type: Option<String>,
    pub size: Option<u64>,
    pub checksum: Option<String>,
}

pub struct StoredArtifact {
    pub stream: ByteStream,
    pub meta: StorageMeta,
}

/// Stores and retrieves cached artifact bytes (the actual file content).
///
/// Keys are opaque strings (typically a `PackageId::cache_key()` prefixed with `"artifact:"`).
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Store artifact bytes. Overwrites any existing entry at `key`.
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError>;

    /// Retrieve an artifact as a streaming response. Returns `None` if not cached.
    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError>;

    /// Check existence without reading data.
    async fn exists(&self, key: &str) -> Result<bool, CoreError>;

    /// Remove a cached artifact.
    async fn delete(&self, key: &str) -> Result<(), CoreError>;

    /// Remove all artifacts whose keys start with `prefix` and return the count deleted.
    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError>;

    /// Count artifacts and sum their sizes for keys starting with `prefix`.
    /// Returns `(count, total_bytes)`.
    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError>;
}
