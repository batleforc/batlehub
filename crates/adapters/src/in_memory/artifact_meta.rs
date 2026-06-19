use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use batlehub_core::{
    error::CoreError,
    ports::{ArtifactCacheMeta, ArtifactInventory, ArtifactMeta, ArtifactMetaRecord},
};

/// A no-op artifact-meta repository that discards all writes and returns
/// empty / non-expired results for all reads. Implements both
/// [`ArtifactCacheMeta`] and [`ArtifactInventory`] (so it satisfies the full
/// `ArtifactMetaRepository` supertrait too).
///
/// Appropriate for tests that exercise proxy or publish paths but do not
/// need eviction or cache-coherence checks.
#[derive(Debug, Default)]
pub struct NoopArtifactMetaRepository;

impl NoopArtifactMetaRepository {
    /// Returns the concrete type behind an `Arc`; it coerces to whichever of the
    /// artifact-meta `dyn` traits the caller's field requires.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self)
    }
}

#[async_trait]
impl ArtifactCacheMeta for NoopArtifactMetaRepository {
    async fn record_artifact(&self, _rec: ArtifactMetaRecord<'_>) -> Result<(), CoreError> {
        Ok(())
    }

    async fn get_artifact_checksum(&self, _key: &str) -> Result<Option<String>, CoreError> {
        Ok(None)
    }

    async fn touch_artifact(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }

    async fn is_artifact_expired(
        &self,
        _key: &str,
        _older_than: DateTime<Utc>,
    ) -> Result<bool, CoreError> {
        Ok(false)
    }

    async fn delete_artifact_meta(&self, _key: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

#[async_trait]
impl ArtifactInventory for NoopArtifactMetaRepository {
    async fn list_artifacts(&self, _registry: &str) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
    }

    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> {
        Ok(vec![])
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn all_methods_return_defaults() {
        let repo = NoopArtifactMetaRepository::arc();

        assert!(repo
            .record_artifact(ArtifactMetaRecord {
                key: "k",
                registry: "cargo",
                package_name: "tokio",
                version: "1.0",
                size: Some(100),
                checksum: None,
            })
            .await
            .is_ok());
        assert!(repo.get_artifact_checksum("k").await.unwrap().is_none());
        assert!(repo.touch_artifact("k").await.is_ok());
        assert!(repo.list_artifacts("cargo").await.unwrap().is_empty());
        assert!(repo.list_artifacts_by_package().await.unwrap().is_empty());
        assert!(repo.delete_artifact_meta("k").await.is_ok());
        assert!(!repo.is_artifact_expired("k", Utc::now()).await.unwrap());
        assert!(repo
            .list_expired_by_ttl("cargo", Utc::now())
            .await
            .unwrap()
            .is_empty());
        assert!(repo
            .list_idle("cargo", Utc::now())
            .await
            .unwrap()
            .is_empty());
        assert_eq!(repo.total_size_bytes("cargo").await.unwrap(), 0);
        assert!(repo.list_lru("cargo", 10).await.unwrap().is_empty());
    }
}
