use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use batlehub_core::{
    error::CoreError,
    ports::{ArtifactMeta, ArtifactMetaRepository},
};

/// A no-op [`ArtifactMetaRepository`] that discards all writes and returns
/// empty / non-expired results for all reads.
///
/// Appropriate for tests that exercise proxy or publish paths but do not
/// need eviction or cache-coherence checks.
#[derive(Debug, Default)]
pub struct NoopArtifactMetaRepository;

impl NoopArtifactMetaRepository {
    pub fn arc() -> Arc<dyn ArtifactMetaRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl ArtifactMetaRepository for NoopArtifactMetaRepository {
    async fn record_artifact(
        &self,
        _key: &str,
        _registry: &str,
        _package_name: &str,
        _version: &str,
        _size: Option<u64>,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    async fn touch_artifact(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn list_artifacts(&self, _registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn delete_artifact_meta(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn is_artifact_expired(
        &self,
        _key: &str,
        _older_than: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn list_expired_by_ttl(
        &self,
        _registry: &str,
        _older_than: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn list_idle(
        &self,
        _registry: &str,
        _idle_since: DateTime<Utc>,
    ) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn total_size_bytes(&self, _registry: &str) -> Result<u64, CoreError> {
        Ok(0)
    }

    async fn list_lru(&self, _registry: &str, _limit: i64) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }
}
