use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream;
use tokio::io::AsyncWriteExt;

use batlehub_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoredArtifact},
};

/// Stores cached artifacts on the local filesystem.
///
/// Each artifact is stored as a single `.dat` file. The key is sanitised:
/// `:` → `__`, `/` stays as the path separator, and `.dat` is appended.
/// The `.dat` suffix prevents a file at `…/v1.2.3.dat` from colliding with
/// the directory `…/v1.2.3/` needed when a sub-artifact (e.g. `.mod`) is
/// stored alongside the version info under the same version prefix.
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
        let rel = key.replace(':', "__");
        self.root.join(format!("{rel}.dat"))
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

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let fs_rel = prefix.replace(':', "__");
        let dir = self.root.join(fs_rel.trim_end_matches('/'));

        let mut count = 0u64;
        let mut total_bytes = 0u64;
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&d).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let path = entry.path();
                if let Ok(ftype) = entry.file_type().await {
                    if ftype.is_dir() {
                        stack.push(path);
                    } else if path.extension().and_then(|e| e.to_str()) == Some("dat") {
                        count += 1;
                        if let Ok(meta) = tokio::fs::metadata(&path).await {
                            total_bytes += meta.len();
                        }
                    }
                }
            }
        }
        Ok((count, total_bytes))
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let fs_rel = prefix.replace(':', "__");
        let dir = self.root.join(fs_rel.trim_end_matches('/'));

        tracing::info!(dir = %dir.display(), prefix = %prefix, "delete_by_prefix: scanning directory");

        // Count .dat files before removing so we can return a meaningful number.
        let mut count = 0usize;
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&d).await {
                Ok(rd) => rd,
                Err(e) => {
                    tracing::warn!(dir = %d.display(), error = %e, "delete_by_prefix: read_dir failed");
                    continue;
                }
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Ok(ftype) = entry.file_type().await {
                    if ftype.is_dir() {
                        stack.push(entry.path());
                    } else if entry.path().extension().and_then(|e| e.to_str()) == Some("dat") {
                        count += 1;
                    }
                }
            }
        }

        tracing::info!(dir = %dir.display(), count, "delete_by_prefix: removing directory");

        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(count.max(1)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => {
                tracing::error!(dir = %dir.display(), error = %e, "delete_by_prefix: remove_dir_all failed");
                Err(CoreError::Storage(e.to_string()))
            }
        }
    }
}
