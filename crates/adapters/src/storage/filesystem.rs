use std::path::PathBuf;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{stream, StreamExt};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use batlehub_core::{
    error::CoreError,
    ports::{ByteStream, StorageBackend, StorageMeta, StoreOutcome, StoredArtifact},
};

use super::read_chunked;

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

    fn key_to_path(&self, key: &str) -> Result<PathBuf, CoreError> {
        crate::storage::ensure_safe_key(key)?;
        let rel = key.replace(':', "__");
        Ok(self.root.join(format!("{rel}.dat")))
    }
}

#[async_trait]
impl StorageBackend for FilesystemStorageBackend {
    async fn store(&self, key: &str, data: Bytes, _meta: StorageMeta) -> Result<(), CoreError> {
        let path = self.key_to_path(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CoreError::Storage(format!("create dirs for {}: {e}", path.display()))
            })?;
        }
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| CoreError::Storage(format!("create file {}: {e}", path.display())))?;
        file.write_all(&data)
            .await
            .map_err(|e| CoreError::Storage(format!("write file {}: {e}", path.display())))?;
        tracing::debug!(key = %key, bytes = data.len(), "stored artifact on filesystem");
        Ok(())
    }

    /// Stream the bytes to disk, hashing as we go. Peak memory is one chunk: the
    /// SHA-256 is computed incrementally rather than over a buffered copy.
    async fn store_streaming(
        &self,
        key: &str,
        mut stream: ByteStream,
        _meta: StorageMeta,
    ) -> Result<StoreOutcome, CoreError> {
        let path = self.key_to_path(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CoreError::Storage(format!("create dirs for {}: {e}", path.display()))
            })?;
        }
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|e| CoreError::Storage(format!("create file {}: {e}", path.display())))?;

        // Write the stream to disk, hashing incrementally. On any mid-stream
        // failure (e.g. an upstream error or a size-limit abort surfaced through
        // the stream) the partially-written file is removed so a later
        // `retrieve` can never serve a truncated artifact.
        let mut hasher = Sha256::new();
        let mut size: u64 = 0;
        let write_result: Result<(), CoreError> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                hasher.update(&chunk);
                size += chunk.len() as u64;
                file.write_all(&chunk).await.map_err(|e| {
                    CoreError::Storage(format!("write file {}: {e}", path.display()))
                })?;
            }
            file.flush()
                .await
                .map_err(|e| CoreError::Storage(format!("flush file {}: {e}", path.display())))
        }
        .await;

        if let Err(e) = write_result {
            drop(file);
            if let Err(rm) = tokio::fs::remove_file(&path).await {
                if rm.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = %path.display(), error = %rm, "failed to remove partial artifact after store_streaming error");
                }
            }
            return Err(e);
        }

        tracing::debug!(key = %key, bytes = size, "streamed artifact to filesystem");
        Ok(StoreOutcome {
            content_hash: hex::encode(hasher.finalize()),
            size,
        })
    }

    /// Atomic rename within the same root — no bytes move through memory.
    async fn move_key(&self, from: &str, to: &str) -> Result<(), CoreError> {
        let from_path = self.key_to_path(from)?;
        let to_path = self.key_to_path(to)?;
        if let Some(parent) = to_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                CoreError::Storage(format!("create dirs for {}: {e}", to_path.display()))
            })?;
        }
        tokio::fs::rename(&from_path, &to_path).await.map_err(|e| {
            CoreError::Storage(format!(
                "rename {} -> {}: {e}",
                from_path.display(),
                to_path.display()
            ))
        })
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let path = self.key_to_path(key)?;
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(CoreError::Storage(format!(
                    "open file {}: {e}",
                    path.display()
                )))
            }
        };
        let size = file.metadata().await.ok().map(|m| m.len());

        // Stream the file off disk in fixed-size chunks; peak memory is one chunk.
        // A zero-length file still yields exactly one (empty) chunk so consumers
        // that expect at least one item behave as they did before streaming.
        let stream: ByteStream = if size == Some(0) {
            Box::pin(stream::once(async { Ok(Bytes::new()) }))
        } else {
            read_chunked(file, path.display().to_string())
        };

        Ok(Some(StoredArtifact {
            stream,
            meta: StorageMeta {
                size,
                ..Default::default()
            },
        }))
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.key_to_path(key)?.exists())
    }

    async fn delete(&self, key: &str) -> Result<bool, CoreError> {
        let path = self.key_to_path(key)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CoreError::Storage(format!(
                "delete file {}: {e}",
                path.display()
            ))),
        }
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        crate::storage::ensure_safe_key(prefix)?;
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
                let Ok(ftype) = entry.file_type().await else {
                    continue;
                };
                if ftype.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("dat") {
                    continue;
                }
                count += 1;
                if let Ok(meta) = tokio::fs::metadata(&path).await {
                    total_bytes += meta.len();
                }
            }
        }
        Ok((count, total_bytes))
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        crate::storage::ensure_safe_key(prefix)?;
        let fs_rel = prefix.replace(':', "__");
        let dir = self.root.join(fs_rel.trim_end_matches('/'));

        let mut keys = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&d).await {
                Ok(rd) => rd,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = rd.next_entry().await {
                let path = entry.path();
                let Ok(ftype) = entry.file_type().await else {
                    continue;
                };
                if ftype.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("dat") {
                    continue;
                }
                // Reconstruct the logical key from the filesystem path.
                let Ok(rel) = path.strip_prefix(&self.root) else {
                    continue;
                };
                let key = rel
                    .to_string_lossy()
                    .trim_end_matches(".dat")
                    .replace("__", ":")
                    .replace(std::path::MAIN_SEPARATOR, "/");
                keys.push(key);
            }
        }
        Ok(keys)
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        crate::storage::ensure_safe_key(prefix)?;
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
                let path = entry.path();
                let Ok(ftype) = entry.file_type().await else {
                    continue;
                };
                if ftype.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) == Some("dat") {
                    count += 1;
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
        b.store(
            "artifact:npm/test-pkg",
            data.clone(),
            StorageMeta::default(),
        )
        .await
        .unwrap();
        let artifact = b
            .retrieve("artifact:npm/test-pkg")
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(collect(artifact).await, b"hello, fs");
    }

    #[tokio::test]
    async fn retrieve_missing_key_returns_none() {
        let b = make_backend().await;
        assert!(b.retrieve("artifact:npm/missing").await.unwrap().is_none());
    }

    fn chunked_stream(chunks: &[&'static [u8]]) -> ByteStream {
        let items: Vec<Result<Bytes, CoreError>> =
            chunks.iter().map(|c| Ok(Bytes::from_static(c))).collect();
        Box::pin(stream::iter(items))
    }

    #[tokio::test]
    async fn store_streaming_round_trips_and_hashes() {
        let b = make_backend().await;
        // "hello" split across chunks; SHA-256 of b"hello".
        let outcome = b
            .store_streaming(
                "artifact:npm/streamed",
                chunked_stream(&[b"he", b"", b"llo"]),
                StorageMeta::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            outcome.content_hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(outcome.size, 5);

        let artifact = b
            .retrieve("artifact:npm/streamed")
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(collect(artifact).await, b"hello");
    }

    #[tokio::test]
    async fn retrieve_streams_large_payload_in_chunks() {
        let b = make_backend().await;
        // Bigger than READ_CHUNK so retrieve yields multiple chunks.
        let big = vec![0xABu8; crate::storage::READ_CHUNK * 2 + 123];
        b.store(
            "artifact:npm/big",
            Bytes::from(big.clone()),
            StorageMeta::default(),
        )
        .await
        .unwrap();
        let artifact = b.retrieve("artifact:npm/big").await.unwrap().unwrap();
        let mut stream = artifact.stream;
        let mut chunks = 0;
        let mut total = Vec::new();
        while let Some(c) = stream.next().await {
            let c = c.unwrap();
            chunks += 1;
            total.extend_from_slice(&c);
        }
        assert_eq!(total, big);
        assert!(
            chunks >= 2,
            "expected a chunked read, got {chunks} chunk(s)"
        );
    }

    #[tokio::test]
    async fn move_key_renames_blob() {
        let b = make_backend().await;
        b.store(
            "blob/staging/abc",
            Bytes::from_static(b"promote me"),
            StorageMeta::default(),
        )
        .await
        .unwrap();
        b.move_key("blob/staging/abc", "blob/deadbeef")
            .await
            .unwrap();
        assert!(!b.exists("blob/staging/abc").await.unwrap());
        let artifact = b.retrieve("blob/deadbeef").await.unwrap().unwrap();
        assert_eq!(collect(artifact).await, b"promote me");
    }

    #[tokio::test]
    async fn retrieve_empty_file_yields_one_empty_chunk() {
        let b = make_backend().await;
        b.store("artifact:npm/empty", Bytes::new(), StorageMeta::default())
            .await
            .unwrap();
        let artifact = b.retrieve("artifact:npm/empty").await.unwrap().unwrap();
        let mut stream = artifact.stream;
        let mut chunks = 0;
        let mut total = Vec::new();
        while let Some(c) = stream.next().await {
            chunks += 1;
            total.extend_from_slice(&c.unwrap());
        }
        assert!(total.is_empty());
        assert_eq!(chunks, 1, "empty file should still yield exactly one chunk");
    }

    #[tokio::test]
    async fn store_streaming_cleans_up_partial_file_on_error() {
        let b = make_backend().await;
        // A stream that yields some bytes, then errors mid-way.
        let items: Vec<Result<Bytes, CoreError>> = vec![
            Ok(Bytes::from_static(b"partial")),
            Err(CoreError::Registry("boom".into())),
        ];
        let stream: ByteStream = Box::pin(stream::iter(items));
        let res = b
            .store_streaming("artifact:npm/aborted", stream, StorageMeta::default())
            .await;
        assert!(res.is_err());
        // No truncated file must be left behind at the key.
        assert!(!b.exists("artifact:npm/aborted").await.unwrap());
        assert!(b.retrieve("artifact:npm/aborted").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn exists_before_and_after_store() {
        let b = make_backend().await;
        assert!(!b.exists("artifact:npm/ex-pkg").await.unwrap());
        b.store(
            "artifact:npm/ex-pkg",
            Bytes::from_static(b"data"),
            StorageMeta::default(),
        )
        .await
        .unwrap();
        assert!(b.exists("artifact:npm/ex-pkg").await.unwrap());
    }

    #[tokio::test]
    async fn delete_removes_file() {
        let b = make_backend().await;
        b.store(
            "artifact:npm/del-pkg",
            Bytes::from_static(b"bye"),
            StorageMeta::default(),
        )
        .await
        .unwrap();
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
        b.store(
            "artifact:npm/colon-test",
            Bytes::from_static(b"x"),
            StorageMeta::default(),
        )
        .await
        .unwrap();
        assert!(b.exists("artifact:npm/colon-test").await.unwrap());
    }

    #[tokio::test]
    async fn stat_by_prefix_counts_and_sums_sizes() {
        let b = make_backend().await;
        for i in 0..3u8 {
            b.store(
                &format!("artifact:npm/stat-pkg-{i}"),
                Bytes::from(vec![i; 100]),
                StorageMeta {
                    size: Some(100),
                    ..Default::default()
                },
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
        b.store(
            "artifact:cargo/keep",
            Bytes::from_static(b"keep"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

        let deleted = b.delete_by_prefix("artifact:npm/").await.unwrap();
        assert!(deleted >= 3, "at least 3 npm artifacts should be deleted");

        let (remaining, _) = b.stat_by_prefix("artifact:npm/").await.unwrap();
        assert_eq!(remaining, 0, "no npm artifacts should remain");

        assert!(
            b.exists("artifact:cargo/keep").await.unwrap(),
            "cargo artifact must survive"
        );
    }

    #[tokio::test]
    async fn delete_by_prefix_nonexistent_returns_zero() {
        let b = make_backend().await;
        let deleted = b.delete_by_prefix("artifact:nonexistent/").await.unwrap();
        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn rejects_path_traversal_keys() {
        let b = make_backend().await;
        // A key whose `..` segments would escape the storage root.
        let evil = "local:npm/../../../../tmp/batlehub-traversal-probe/1.0";

        let store_err = b
            .store(evil, Bytes::from_static(b"x"), StorageMeta::default())
            .await;
        assert!(matches!(store_err, Err(CoreError::InvalidInput(_))));

        assert!(matches!(
            b.retrieve(evil).await,
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            b.delete(evil).await,
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            b.exists(evil).await,
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            b.delete_by_prefix("local:npm/../../../../tmp").await,
            Err(CoreError::InvalidInput(_))
        ));

        // Nothing was written outside the root.
        assert!(
            !std::path::Path::new("/tmp/batlehub-traversal-probe").exists(),
            "traversal must not create files outside the storage root"
        );
    }
}
