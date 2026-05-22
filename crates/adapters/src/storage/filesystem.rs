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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use bytes::Bytes;
    use futures::StreamExt;

    use super::*;

    static DIR_ID: AtomicU64 = AtomicU64::new(0);

    async fn make_backend() -> FilesystemStorageBackend {
        let id = DIR_ID.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("batlehub-test-fs-{pid}-{id}"));
        FilesystemStorageBackend::new(dir).await.unwrap()
    }

    async fn collect(artifact: StoredArtifact) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut stream = artifact.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk.unwrap());
        }
        buf
    }

    #[tokio::test]
    async fn store_and_retrieve_round_trip() {
        let b = make_backend().await;
        let data = Bytes::from_static(b"hello, fs");
        b.store("artifact:npm/test-pkg", data.clone(), StorageMeta::default()).await.unwrap();
        let artifact = b.retrieve("artifact:npm/test-pkg").await.unwrap().expect("should exist");
        assert_eq!(collect(artifact).await, b"hello, fs");
    }

    #[tokio::test]
    async fn retrieve_missing_key_returns_none() {
        let b = make_backend().await;
        assert!(b.retrieve("artifact:npm/missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn exists_before_and_after_store() {
        let b = make_backend().await;
        assert!(!b.exists("artifact:npm/ex-pkg").await.unwrap());
        b.store("artifact:npm/ex-pkg", Bytes::from_static(b"data"), StorageMeta::default()).await.unwrap();
        assert!(b.exists("artifact:npm/ex-pkg").await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let b = make_backend().await;
        b.store("artifact:npm/del-pkg", Bytes::from_static(b"bye"), StorageMeta::default()).await.unwrap();
        b.delete("artifact:npm/del-pkg").await.unwrap();
        assert!(!b.exists("artifact:npm/del-pkg").await.unwrap());
    }

    #[tokio::test]
    async fn delete_missing_key_is_ok() {
        let b = make_backend().await;
        b.delete("artifact:npm/ghost").await.unwrap();
    }

    #[tokio::test]
    async fn colon_in_key_is_stored_and_retrieved() {
        let b = make_backend().await;
        b.store("artifact:npm/colon-test", Bytes::from_static(b"x"), StorageMeta::default()).await.unwrap();
        assert!(b.exists("artifact:npm/colon-test").await.unwrap());
    }

    #[tokio::test]
    async fn stat_by_prefix_counts_and_sums_sizes() {
        let b = make_backend().await;
        for i in 0..3u8 {
            b.store(
                &format!("artifact:npm/stat-pkg-{i}"),
                Bytes::from(vec![i; 100]),
                StorageMeta { size: Some(100), ..Default::default() },
            )
            .await
            .unwrap();
        }
        // Prefix must end with '/' to resolve to the registry directory.
        let (count, bytes) = b.stat_by_prefix("artifact:npm/").await.unwrap();
        assert_eq!(count, 3);
        assert_eq!(bytes, 300);
    }

    #[tokio::test]
    async fn stat_by_prefix_returns_zero_for_nonexistent_prefix() {
        let b = make_backend().await;
        let (count, bytes) = b.stat_by_prefix("artifact:nonexistent/").await.unwrap();
        assert_eq!(count, 0);
        assert_eq!(bytes, 0);
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_all_matching_files() {
        let b = make_backend().await;
        for i in 0..3u8 {
            b.store(
                &format!("artifact:npm/del-pkg-{i}"),
                Bytes::from(vec![0u8; 10]),
                StorageMeta::default(),
            )
            .await
            .unwrap();
        }
        b.store("artifact:cargo/keep", Bytes::from_static(b"keep"), StorageMeta::default()).await.unwrap();

        let deleted = b.delete_by_prefix("artifact:npm/").await.unwrap();
        assert!(deleted >= 3, "at least 3 npm artifacts should be deleted");

        let (remaining, _) = b.stat_by_prefix("artifact:npm/").await.unwrap();
        assert_eq!(remaining, 0, "no npm artifacts should remain");

        assert!(b.exists("artifact:cargo/keep").await.unwrap(), "cargo artifact must survive");
    }

    #[tokio::test]
    async fn delete_by_prefix_nonexistent_returns_zero() {
        let b = make_backend().await;
        let deleted = b.delete_by_prefix("artifact:nonexistent/").await.unwrap();
        assert_eq!(deleted, 0);
    }
}
