use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sha2::Digest;
use sqlx::PgPool;

use batlehub_core::{
    error::CoreError,
    ports::{StorageBackend, StorageMeta, StoredArtifact},
};

mod routing;
mod tracking;

pub use routing::route_key_to_backend;

/// Routes artifact storage operations across multiple named backends, with
/// content-addressable deduplication via the `artifact_dedup_index` and
/// `artifact_dedup_refs` tables.
///
/// **Dedup model:**
/// - Physical blobs are stored under `"blob/{sha256_hex}"` in the selected backend.
/// - `artifact_dedup_refs` maps each logical artifact key to a content hash.
/// - `artifact_dedup_index` maps each content hash to its physical key and ref count.
/// - When two logical keys share identical bytes, only one physical blob is written.
/// - Deleting a logical key decrements the ref count; the blob is deleted only when
///   the ref count reaches zero.
/// - Artifacts written before this feature was enabled have no dedup entries and are
///   served via the legacy path (direct logical-key lookup).
pub struct StorageRouter {
    pub(super) backends: HashMap<String, Arc<dyn StorageBackend>>,
    pub(super) default_name: String,
    /// registry_name → backend_name from RegistryConfig.storage
    pub(super) registry_assignments: HashMap<String, String>,
    pub(super) pool: PgPool,
}

impl StorageRouter {
    pub fn new(
        backends: HashMap<String, Arc<dyn StorageBackend>>,
        default_name: String,
        registry_assignments: HashMap<String, String>,
        pool: PgPool,
    ) -> Self {
        Self {
            backends,
            default_name,
            registry_assignments,
            pool,
        }
    }

    fn backend_name_for_key(&self, key: &str) -> &str {
        routing::route_key_to_backend(key, &self.registry_assignments, &self.default_name)
    }

    pub(super) fn resolve_backend(&self, name: &str) -> &Arc<dyn StorageBackend> {
        self.backends
            .get(name)
            .or_else(|| self.backends.get(&self.default_name))
            .expect("default storage backend must always be present")
    }
}

#[async_trait]
impl StorageBackend for StorageRouter {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        let content_hash = hex::encode(sha2::Sha256::digest(&data));
        let content_key = format!("blob/{content_hash}");
        let size = meta.size;

        let backend_name = self.backend_name_for_key(key).to_owned();
        let backend = self.resolve_backend(&backend_name).clone();

        // Atomically update the dedup tables inside a single transaction so that a
        // failed refs insert can never leave an orphaned ref-count increment.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CoreError::Storage(format!("failed to begin transaction: {e}")))?;

        // Check whether this logical key already maps to a content hash.  A
        // re-store of identical bytes for the same key must NOT increment ref_count.
        let existing_hash: Option<String> = sqlx::query_scalar(
            "SELECT content_hash FROM artifact_dedup_refs WHERE logical_key = $1 FOR UPDATE",
        )
        .bind(key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        if existing_hash.as_deref() == Some(content_hash.as_str()) {
            // Identical bytes re-stored under the same key — nothing to do in the DB.
            tx.rollback().await.ok();
        } else {
            // Decrement ref count for the previous hash if the key is being replaced.
            if let Some(old_hash) = &existing_hash {
                let old_count: i32 = sqlx::query_scalar(
                    "UPDATE artifact_dedup_index SET ref_count = ref_count - 1 \
                     WHERE content_hash = $1 RETURNING ref_count",
                )
                .bind(old_hash)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| CoreError::Storage(e.to_string()))?;

                if old_count <= 0 {
                    sqlx::query("DELETE FROM artifact_dedup_index WHERE content_hash = $1")
                        .bind(old_hash)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| CoreError::Storage(e.to_string()))?;
                }
            }

            // Increment (or insert) ref count for the new hash.
            let count: i32 = sqlx::query_scalar(
                r#"
                INSERT INTO artifact_dedup_index (content_hash, content_key, ref_count, size_bytes)
                VALUES ($1, $2, 1, $3)
                ON CONFLICT (content_hash) DO UPDATE
                    SET ref_count = artifact_dedup_index.ref_count + 1
                RETURNING ref_count
                "#,
            )
            .bind(&content_hash)
            .bind(&content_key)
            .bind(size.map(|s| s as i64))
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| CoreError::Storage(format!("dedup index upsert failed: {e}")))?;

            // Map logical key → content hash (propagate errors instead of silently dropping).
            sqlx::query(
                r#"
                INSERT INTO artifact_dedup_refs (logical_key, content_hash)
                VALUES ($1, $2)
                ON CONFLICT (logical_key) DO UPDATE SET content_hash = EXCLUDED.content_hash
                "#,
            )
            .bind(key)
            .bind(&content_hash)
            .execute(&mut *tx)
            .await
            .map_err(|e| CoreError::Storage(format!("dedup refs insert failed: {e}")))?;

            // Write the physical blob while the transaction is still open.  Doing
            // this before commit ensures a backend failure causes a full rollback
            // rather than leaving orphaned dedup rows that point to a missing blob.
            if count == 1 {
                if let Err(e) = backend.store(&content_key, data, meta).await {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }

            tx.commit()
                .await
                .map_err(|e| CoreError::Storage(e.to_string()))?;
        }

        // Keep the legacy artifact_storage record for routing and size queries.
        self.record_backend(key, &backend_name, size).await;

        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        // Dedup path: resolve logical key → physical content key.
        if let Some((content_key, backend)) = self.dedup_content_key(key).await {
            let artifact = backend.retrieve(&content_key).await?;
            if artifact.is_some() {
                return Ok(artifact);
            }
            // Physical blob missing (possible race during first write); fall through.
        }

        // Legacy path: artifact was stored before dedup was enabled.
        let backend = match self.recorded_backend_for_key(key).await {
            Some(b) => b,
            None => self.resolve_backend(self.backend_name_for_key(key)).clone(),
        };
        let artifact = backend.retrieve(key).await?;
        if let Some(ref a) = artifact {
            if let Some(size) = a.meta.size {
                self.lazy_update_size(key, size).await;
            }
        }
        Ok(artifact)
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        // Dedup path.
        if let Some((content_key, backend)) = self.dedup_content_key(key).await {
            return backend.exists(&content_key).await;
        }

        // Legacy path.
        match self.recorded_backend_for_key(key).await {
            Some(b) => b.exists(key).await,
            None => {
                self.resolve_backend(self.backend_name_for_key(key))
                    .exists(key)
                    .await
            }
        }
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        // ── Dedup path ────────────────────────────────────────────────────────
        // Delegates the heavy DB work to `delete_dedup_entry` in tracking.rs.
        if let Some((should_delete_blob, content_key, blob_backend_name)) =
            self.delete_dedup_entry(key).await?
        {
            // Delete physical blob when no more references.
            if should_delete_blob {
                let backend = self.resolve_backend(&blob_backend_name);
                let _ = backend.delete(&content_key).await.inspect_err(
                    |e| tracing::warn!(error = %e, key = %content_key, "failed to delete physical blob"),
                );
            }
        } else {
            // ── Legacy path ───────────────────────────────────────────────────
            let backend = match self.recorded_backend_for_key(key).await {
                Some(b) => b,
                None => self.resolve_backend(self.backend_name_for_key(key)).clone(),
            };
            backend.delete(key).await?;
        }

        // Clean up the routing record regardless of path.
        let _ = sqlx::query("DELETE FROM artifact_storage WHERE storage_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .inspect_err(
                |e| tracing::warn!(error = %e, key, "failed to delete artifact_storage record"),
            );

        Ok(())
    }

    /// Returns count and total size of all logical artifact keys matching `prefix`.
    /// Queries the DB tables rather than the physical backend (which now stores
    /// content-addressed blobs under `blob/` prefixes).
    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        use sqlx::Row;
        let like = format!("{prefix}%");
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) AS cnt, COALESCE(SUM(size_bytes), 0) AS total
            FROM artifact_storage
            WHERE storage_key LIKE $1
            "#,
        )
        .bind(&like)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CoreError::Storage(e.to_string()))?;

        let count: i64 = row.try_get("cnt").unwrap_or(0);
        let total: i64 = row.try_get("total").unwrap_or(0);
        Ok((count as u64, total as u64))
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let keys = self.logical_keys_by_prefix(prefix).await?;
        let count = keys.len();
        for key in keys {
            if let Err(e) = self.delete(&key).await {
                tracing::warn!(error = %e, key, "delete_by_prefix: failed to delete artifact");
            }
        }
        Ok(count)
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        self.logical_keys_by_prefix(prefix).await
    }
}
