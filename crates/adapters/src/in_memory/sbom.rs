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

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::entities::SbomSource;

    fn sbom(key: &str, registry: &str, name: &str, version: &str, fmt: SbomFormat) -> ArtifactSbom {
        ArtifactSbom {
            id: uuid::Uuid::new_v4(),
            artifact_key: key.into(),
            registry: registry.into(),
            package_name: name.into(),
            version: version.into(),
            format: fmt,
            spec_version: "1.0".into(),
            document: serde_json::json!({}),
            source: SbomSource::Generated,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn noop_repo_stores_nothing() {
        let repo = NoopSbomRepository::arc();
        repo.upsert_sbom(sbom("k", "npm", "p", "1", SbomFormat::Spdx))
            .await
            .unwrap();
        assert!(repo
            .get_sbom("k", &SbomFormat::Spdx)
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .get_sbom_by_coordinates("npm", "p", "1", &SbomFormat::Spdx)
            .await
            .unwrap()
            .is_none());
        assert!(repo
            .list_sboms_for_export(None, None, None, 10, 0)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn upsert_replaces_same_key_and_format() {
        let repo = InMemorySbomRepository::new();
        repo.upsert_sbom(sbom("k", "npm", "p", "1", SbomFormat::Spdx))
            .await
            .unwrap();
        // Same key+format → replace (not duplicate).
        repo.upsert_sbom(sbom("k", "npm", "p", "2", SbomFormat::Spdx))
            .await
            .unwrap();
        // Different format → coexists.
        repo.upsert_sbom(sbom("k", "npm", "p", "1", SbomFormat::CycloneDx))
            .await
            .unwrap();

        let got = repo
            .get_sbom("k", &SbomFormat::Spdx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.version, "2");
        assert!(repo
            .get_sbom("k", &SbomFormat::CycloneDx)
            .await
            .unwrap()
            .is_some());
        assert!(repo
            .get_sbom("missing", &SbomFormat::Spdx)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn lookup_by_coordinates_and_export_filtering() {
        let repo = InMemorySbomRepository::new();
        repo.upsert_sbom(sbom("k1", "npm", "a", "1", SbomFormat::Spdx))
            .await
            .unwrap();
        repo.upsert_sbom(sbom("k2", "cargo", "b", "2", SbomFormat::Spdx))
            .await
            .unwrap();

        assert_eq!(
            repo.get_sbom_by_coordinates("npm", "a", "1", &SbomFormat::Spdx)
                .await
                .unwrap()
                .unwrap()
                .artifact_key,
            "k1"
        );
        // Registry filter narrows the export; limit/offset paginate.
        assert_eq!(
            repo.list_sboms_for_export(Some("npm"), None, None, 10, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            repo.list_sboms_for_export(None, None, None, 10, 0)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            repo.list_sboms_for_export(None, None, None, 1, 1)
                .await
                .unwrap()
                .len(),
            1
        );
    }
}
