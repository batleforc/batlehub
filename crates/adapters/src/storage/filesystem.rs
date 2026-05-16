use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream;
use tokio::io::AsyncWriteExt;

use proxy_cache_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoredArtifact},
};

/// Stores cached artifacts on the local filesystem.
///
/// Each artifact is stored as a single file. The key is sanitised to a path
/// by replacing `:` and the path separator with `__` and `/` respectively.
pub struct FilesystemStorageBackend {
    root: PathBuf,
}

impl FilesystemStorageBackend {
    pub async fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self { root })
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        // key example: "artifact:github/rust-lang/rust/v1.80.0/12345"
        // strip the "artifact:" prefix and use the rest as a relative path.
        let rel = key.replace(':', "__");
        self.root.join(rel)
    }
}

#[async_trait]
impl StorageBackend for FilesystemStorageBackend {
    async fn store(&self, key: &str, data: Bytes, _meta: StorageMeta) -> Result<(), CoreError> {
        let path = self.key_to_path(key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CoreError::Storage(format!("create dirs for {}: {e}", path.display()))
            })?;
        }
        let mut file = tokio::fs::File::create(&path).await.map_err(|e| {
            CoreError::Storage(format!("create file {}: {e}", path.display()))
        })?;
        file.write_all(&data).await.map_err(|e| {
            CoreError::Storage(format!("write file {}: {e}", path.display()))
        })?;
        tracing::debug!(key = %key, bytes = data.len(), "stored artifact on filesystem");
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let path = self.key_to_path(key);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let size = bytes.len() as u64;
                let data = Bytes::from(bytes);
                let stream: ByteStream = Box::pin(stream::once(async move { Ok(data) }));
                Ok(Some(StoredArtifact {
                    stream,
                    meta: StorageMeta {
                        size: Some(size),
                        ..Default::default()
                    },
                }))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::Storage(format!(
                "read file {}: {e}",
                path.display()
            ))),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.key_to_path(key).exists())
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        let path = self.key_to_path(key);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(CoreError::Storage(format!(
                "delete file {}: {e}",
                path.display()
            ))),
        }
    }
}
