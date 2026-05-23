use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sqlx::PgPool;

use batlehub_core::{
    error::CoreError,
    ports::{StorageBackend, StorageMeta, StoredArtifact},
};

/// Routes artifact storage operations across multiple named backends.
///
/// - On `store`: the backend is chosen by registry name (derived from the artifact key).
///   The assignment is recorded in the `artifact_storage` table for future retrieval.
/// - On `retrieve`/`exists`: the recorded backend is consulted first; artifacts without
///   a record (pre-migration) fall back to the default backend.
pub struct StorageRouter {
    backends: HashMap<String, Arc<dyn StorageBackend>>,
    default_name: String,
    /// registry_name → backend_name from RegistryConfig.storage
    registry_assignments: HashMap<String, String>,
    pool: PgPool,
}

impl StorageRouter {
    pub fn new(
        backends: HashMap<String, Arc<dyn StorageBackend>>,
        default_name: String,
        registry_assignments: HashMap<String, String>,
        pool: PgPool,
    ) -> Self {
        Self { backends, default_name, registry_assignments, pool }
    }

    fn resolve_backend_for_key(&self, key: &str) -> &Arc<dyn StorageBackend> {
        let registry = key
            .strip_prefix("artifact:")
            .and_then(|k| k.split('/').next())
            .unwrap_or("");

        let backend_name = self
            .registry_assignments
            .get(registry)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_name);

        self.backends
            .get(backend_name)
            .or_else(|| self.backends.get(&self.default_name))
            .expect("default storage backend must always be present")
    }

    async fn recorded_backend_for_key(&self, key: &str) -> Option<&Arc<dyn StorageBackend>> {
        use sqlx::Row;
        let result = sqlx::query(
            "SELECT backend_name FROM artifact_storage WHERE storage_key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten();

        result
            .and_then(|r| r.try_get::<String, _>("backend_name").ok())
            .and_then(|name| self.backends.get(&name))
    }

    async fn lazy_update_size(&self, key: &str, size: u64) {
        let _ = sqlx::query(
            "UPDATE artifact_storage SET size_bytes = $1 WHERE storage_key = $2 AND size_bytes IS NULL",
        )
        .bind(size as i64)
        .bind(key)
        .execute(&self.pool)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, key, "failed to update artifact size"));
    }

    async fn record_backend(&self, key: &str, backend_name: &str, size_bytes: Option<u64>) {
        let _ = sqlx::query(
            r#"
            INSERT INTO artifact_storage (storage_key, backend_name, stored_at, size_bytes)
            VALUES ($1, $2, NOW(), $3)
            ON CONFLICT (storage_key) DO UPDATE
                SET backend_name = EXCLUDED.backend_name,
                    size_bytes = EXCLUDED.size_bytes
            "#,
        )
        .bind(key)
        .bind(backend_name)
        .bind(size_bytes.map(|s| s as i64))
        .execute(&self.pool)
        .await
        .inspect_err(|e| tracing::warn!(error = %e, key, backend_name, "failed to record artifact backend"));
    }

    fn backend_name_for_key(&self, key: &str) -> &str {
        let registry = key
            .strip_prefix("artifact:")
            .and_then(|k| k.split('/').next())
            .unwrap_or("");

        self.registry_assignments
            .get(registry)
            .map(|s| s.as_str())
            .unwrap_or(&self.default_name)
    }
}

#[async_trait]
impl StorageBackend for StorageRouter {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        let backend_name = self.backend_name_for_key(key).to_owned();
        let backend = self.backends
            .get(&backend_name)
            .or_else(|| self.backends.get(&self.default_name))
            .expect("default storage backend must always be present");

        let size_bytes = meta.size;
        backend.store(key, data, meta).await?;
        self.record_backend(key, &backend_name, size_bytes).await;
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        // Clone the Arc so we don't hold a reference into self across the await.
        let backend = match self.recorded_backend_for_key(key).await {
            Some(b) => Arc::clone(b),
            None => Arc::clone(self.resolve_backend_for_key(key)),
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
        if let Some(backend) = self.recorded_backend_for_key(key).await {
            return backend.exists(key).await;
        }
        self.resolve_backend_for_key(key).exists(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        let backend = if let Some(b) = self.recorded_backend_for_key(key).await {
            b
        } else {
            self.resolve_backend_for_key(key)
        };

        backend.delete(key).await?;

        let _ = sqlx::query("DELETE FROM artifact_storage WHERE storage_key = $1")
            .bind(key)
            .execute(&self.pool)
            .await
            .inspect_err(|e| tracing::warn!(error = %e, key, "failed to delete artifact_storage record"));

        Ok(())
    }

    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let backend = Arc::clone(self.resolve_backend_for_key(prefix));
        backend.stat_by_prefix(prefix).await
    }

    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let backend = Arc::clone(self.resolve_backend_for_key(prefix));
        backend.delete_by_prefix(prefix).await
    }

    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        use std::collections::HashSet;

        let mut seen = HashSet::new();
        let mut all_keys = Vec::new();

        for backend in self.backends.values() {
            let keys = backend.list_keys(prefix).await?;
            for key in keys {
                if seen.insert(key.clone()) {
                    all_keys.push(key);
                }
            }
        }
        Ok(all_keys)
    }
}
