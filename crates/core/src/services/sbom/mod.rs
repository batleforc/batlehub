use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{ArtifactSbom, PackageMetadata, SbomFormat, SbomSource};
use crate::error::CoreError;
use crate::ports::{SbomDependency, SbomExtractor, SbomRepository, UpstreamSbomFetcher};

mod fetch;
mod generate;

pub struct SbomService {
    pub repo: Arc<dyn SbomRepository>,
    pub extractor: Option<Arc<dyn SbomExtractor>>,
    pub fetcher: Option<Arc<dyn UpstreamSbomFetcher>>,
}

/// Options for recording an SBOM when a package is published locally.
pub struct SbomPublishOptions<'a> {
    pub registry_type: &'a str,
    pub formats: &'a [SbomFormat],
    pub required: bool,
}

// ── SbomService implementation ────────────────────────────────────────────────

impl SbomService {
    pub fn new(
        repo: Arc<dyn SbomRepository>,
        extractor: Option<Arc<dyn SbomExtractor>>,
        fetcher: Option<Arc<dyn UpstreamSbomFetcher>>,
    ) -> Self {
        Self {
            repo,
            extractor,
            fetcher,
        }
    }

    /// Store both SPDX and CycloneDX SBOMs for a proxied artifact.
    ///
    /// `registry_type` is the adapter type string (e.g. "cargo", "npm") used for
    /// archive extraction and upstream SBOM fetching — distinct from the user-defined
    /// registry name stored in `meta.id.registry`.
    ///
    /// Priority for SPDX: upstream-fetched > archive-extracted > minimal generated.
    /// CycloneDX is always generated (minimal if no deps extracted).
    pub async fn record_for_proxied(
        &self,
        meta: &PackageMetadata,
        artifact_key: &str,
        data: &Bytes,
        formats: &[SbomFormat],
        fetch_upstream: bool,
        registry_type: &str,
    ) -> Result<(), CoreError> {
        // Try upstream SBOM (SPDX only — GitHub returns SPDX)
        let upstream_spdx = if fetch_upstream {
            if let Some(ref fetcher) = self.fetcher {
                fetcher
                    .fetch(registry_type, &meta.id.name, &meta.id.version)
                    .await
                    .unwrap_or(None)
            } else {
                None
            }
        } else {
            None
        };

        // Extract deps from archive (fallback to empty if no extractor)
        let deps: Vec<SbomDependency> = self
            .extractor
            .as_ref()
            .map(|e| e.extract(data, registry_type))
            .unwrap_or_default();

        let source = if upstream_spdx.is_some() {
            SbomSource::Upstream
        } else if !deps.is_empty() {
            SbomSource::Extracted
        } else {
            SbomSource::Generated
        };

        for format in formats {
            let document = match format {
                SbomFormat::Spdx => upstream_spdx
                    .clone()
                    .unwrap_or_else(|| generate::generate_spdx(meta, artifact_key, &deps)),
                SbomFormat::CycloneDx => generate::generate_cyclonedx(meta, artifact_key, &deps),
            };

            self.repo
                .upsert_sbom(ArtifactSbom {
                    id: Uuid::new_v4(),
                    artifact_key: artifact_key.to_owned(),
                    registry: meta.id.registry.clone(),
                    package_name: meta.id.name.clone(),
                    version: meta.id.version.clone(),
                    spec_version: format.spec_version().to_owned(),
                    format: format.clone(),
                    document,
                    source: source.clone(),
                    created_at: Utc::now(),
                })
                .await?;
        }

        Ok(())
    }

    /// Store SBOMs for a privately published artifact.
    ///
    /// Extracts deps from the archive. When `required` is `true` and no manifest
    /// can be found, returns `CoreError::AccessDenied` so the publish can be rolled back.
    pub async fn record_for_published(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        artifact_key: &str,
        data: &Bytes,
        opts: SbomPublishOptions<'_>,
    ) -> Result<(), CoreError> {
        let SbomPublishOptions {
            registry_type,
            formats,
            required,
        } = opts;
        let deps: Vec<SbomDependency> = self
            .extractor
            .as_ref()
            .map(|e| e.extract(data, registry_type))
            .unwrap_or_default();

        if required && deps.is_empty() {
            return Err(CoreError::AccessDenied(
                "SBOM required: no dependency manifest found in the uploaded archive".into(),
            ));
        }

        let source = if deps.is_empty() {
            SbomSource::Generated
        } else {
            SbomSource::Extracted
        };

        let fake_meta = PackageMetadata {
            id: crate::entities::PackageId::new(registry, name, version),
            published_at: Some(Utc::now()),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        };

        for format in formats {
            let document = match format {
                SbomFormat::Spdx => generate::generate_spdx(&fake_meta, artifact_key, &deps),
                SbomFormat::CycloneDx => {
                    generate::generate_cyclonedx(&fake_meta, artifact_key, &deps)
                }
            };

            self.repo
                .upsert_sbom(ArtifactSbom {
                    id: Uuid::new_v4(),
                    artifact_key: artifact_key.to_owned(),
                    registry: registry.to_owned(),
                    package_name: name.to_owned(),
                    version: version.to_owned(),
                    spec_version: format.spec_version().to_owned(),
                    format: format.clone(),
                    document,
                    source: source.clone(),
                    created_at: Utc::now(),
                })
                .await?;
        }

        Ok(())
    }

    pub async fn get_artifact_sbom(
        &self,
        artifact_key: &str,
        format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError> {
        self.repo.get_sbom(artifact_key, format).await
    }

    /// Merge all SBOMs in the given time range into a single org-level document.
    pub async fn export_org_sbom(
        &self,
        registry: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        format: &SbomFormat,
    ) -> Result<serde_json::Value, CoreError> {
        let sboms = fetch::collect_all_sbom_pages(&*self.repo, registry, from, to).await?;
        match format {
            SbomFormat::Spdx => {
                let (packages, relationships) = fetch::collect_spdx_entries(&sboms);
                Ok(generate::build_spdx_document(packages, relationships))
            }
            SbomFormat::CycloneDx => {
                let components = fetch::collect_cyclonedx_components(&sboms);
                Ok(generate::build_cyclonedx_document(components))
            }
        }
    }
}
