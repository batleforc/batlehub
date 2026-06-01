use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use batlehub_core::{
    entities::{ArtifactSbom, SbomFormat},
    error::CoreError,
    ports::SbomRepository,
};

/// No-op SBOM repository — used in tests and when SBOM is disabled.
pub struct NoopSbomRepository;

impl NoopSbomRepository {
    pub fn arc() -> Arc<dyn SbomRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl SbomRepository for NoopSbomRepository {
    async fn upsert_sbom(&self, _sbom: ArtifactSbom) -> Result<(), CoreError> {
        Ok(())
    }

    async fn get_sbom(
        &self,
        _artifact_key: &str,
        _format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError> {
        Ok(None)
    }

    async fn list_sboms_for_export(
        &self,
        _registry: Option<&str>,
        _from: Option<DateTime<Utc>>,
        _to: Option<DateTime<Utc>>,
        _limit: u64,
        _offset: u64,
    ) -> Result<Vec<ArtifactSbom>, CoreError> {
        Ok(vec![])
    }
}
