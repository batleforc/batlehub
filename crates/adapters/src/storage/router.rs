use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use sqlx::PgPool;

use proxy_cache_core::{
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

    async fn record_backend(&self, key: &str, backend_name: &str) {
        let _ = sqlx::query(
            r#"
            INSERT INTO artifact_storage (storage_key, backend_name, stored_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (storage_key) DO UPDATE SET backend_name = EXCLUDED.backend_name
            "#,
        )
        .bind(key)
        .bind(backend_name)
        .execute(&self.pool)
        .await;
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

        backend.store(key, data, meta).await?;
        self.record_backend(key, &backend_name).await;
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        if let Some(backend) = self.recorded_backend_for_key(key).await {
            return backend.retrieve(key).await;
        }
        // Pre-migration artifact: fall back to default
        self.resolve_backend_for_key(key).retrieve(key).await
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
            .await;

        Ok(())
    }
}
