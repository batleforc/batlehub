use std::sync::{Arc, Mutex};

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

    async fn get_sbom_by_coordinates(
        &self,
        _registry: &str,
        _package_name: &str,
        _version: &str,
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

/// In-memory SBOM repository for tests — stores upserted SBOMs in a `Vec` and
/// supports lookup by artifact key, by coordinates, and export listing.
pub struct InMemorySbomRepository {
    items: Mutex<Vec<ArtifactSbom>>,
}

impl InMemorySbomRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl SbomRepository for InMemorySbomRepository {
    async fn upsert_sbom(&self, sbom: ArtifactSbom) -> Result<(), CoreError> {
        let mut items = self.items.lock().unwrap();
        items.retain(|s| !(s.artifact_key == sbom.artifact_key && s.format == sbom.format));
        items.push(sbom);
        Ok(())
    }

    async fn get_sbom(
        &self,
        artifact_key: &str,
        format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.artifact_key == artifact_key && &s.format == format)
            .cloned())
    }

    async fn get_sbom_by_coordinates(
        &self,
        registry: &str,
        package_name: &str,
        version: &str,
        format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .find(|s| {
                s.registry == registry
                    && s.package_name == package_name
                    && s.version == version
                    && &s.format == format
            })
            .cloned())
    }

    async fn list_sboms_for_export(
        &self,
        registry: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<ArtifactSbom>, CoreError> {
        let items = self.items.lock().unwrap();
        let filtered: Vec<ArtifactSbom> = items
            .iter()
            .filter(|s| registry.is_none_or(|r| s.registry == r))
            .filter(|s| from.is_none_or(|f| s.created_at >= f))
            .filter(|s| to.is_none_or(|t| s.created_at <= t))
            .cloned()
            .collect();
        Ok(filtered
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .collect())
    }
}
