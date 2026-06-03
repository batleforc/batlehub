use std::collections::HashSet;
use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::{ArtifactSbom, PackageMetadata, SbomFormat, SbomSource};
use crate::error::CoreError;
use crate::ports::{SbomDependency, SbomExtractor, SbomRepository, UpstreamSbomFetcher};

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

// ── PURL helpers ──────────────────────────────────────────────────────────────

fn registry_to_purl(registry_type: &str, name: &str, version: &str) -> String {
    match registry_type {
        "cargo" => format!("pkg:cargo/{name}@{version}"),
        "npm" => format!("pkg:npm/{name}@{version}"),
        "maven" => format!("pkg:maven/{name}@{version}"),
        "pypi" => format!("pkg:pypi/{name}@{version}"),
        "rubygems" => format!("pkg:gem/{name}@{version}"),
        "goproxy" => format!("pkg:golang/{name}@{version}"),
        "composer" => format!("pkg:composer/{name}@{version}"),
        "conda" => format!("pkg:conda/{name}@{version}"),
        _ => format!("pkg:generic/{name}@{version}"),
    }
}

// ── SPDX 2.3 JSON generation ──────────────────────────────────────────────────

fn generate_spdx(
    meta: &PackageMetadata,
    artifact_key: &str,
    deps: &[SbomDependency],
) -> serde_json::Value {
    let doc_ns = format!(
        "https://batlehub/sbom/{}/{}/{}/{}",
        meta.id.registry,
        meta.id.name,
        meta.id.version,
        Uuid::new_v4()
    );

    let download_location = meta
        .download_url
        .clone()
        .unwrap_or_else(|| "NOASSERTION".to_owned());

    let mut checksums = serde_json::json!([]);
    if let Some(ref ck) = meta.checksum {
        checksums = serde_json::json!([{"algorithm": "SHA256", "checksumValue": ck}]);
    }

    let mut packages = vec![serde_json::json!({
        "SPDXID": "SPDXRef-Package",
        "name": meta.id.name,
        "versionInfo": meta.id.version,
        "downloadLocation": download_location,
        "filesAnalyzed": false,
        "checksums": checksums,
        "supplier": "NOASSERTION",
        "comment": artifact_key,
    })];

    let mut relationships = vec![serde_json::json!({
        "spdxElementId": "SPDXRef-DOCUMENT",
        "relationshipType": "DESCRIBES",
        "relatedSpdxElement": "SPDXRef-Package",
    })];

    for (i, dep) in deps.iter().enumerate() {
        let dep_id = format!("SPDXRef-Dep-{i}");
        packages.push(serde_json::json!({
            "SPDXID": dep_id,
            "name": dep.name,
            "versionInfo": dep.version_req.as_deref().unwrap_or("NOASSERTION"),
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
        }));
        relationships.push(serde_json::json!({
            "spdxElementId": format!("SPDXRef-Dep-{i}"),
            "relationshipType": "DEPENDENCY_OF",
            "relatedSpdxElement": "SPDXRef-Package",
        }));
    }

    serde_json::json!({
        "spdxVersion": "SPDX-2.3",
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("{}-{}", meta.id.name, meta.id.version),
        "documentNamespace": doc_ns,
        "packages": packages,
        "relationships": relationships,
    })
}

// ── CycloneDX 1.4 JSON generation ─────────────────────────────────────────────

fn generate_cyclonedx(
    meta: &PackageMetadata,
    artifact_key: &str,
    deps: &[SbomDependency],
) -> serde_json::Value {
    let purl = registry_to_purl(&meta.id.registry, &meta.id.name, &meta.id.version);

    let mut hashes = serde_json::json!([]);
    if let Some(ref ck) = meta.checksum {
        hashes = serde_json::json!([{"alg": "SHA-256", "content": ck}]);
    }

    let main_component = serde_json::json!({
        "type": "library",
        "name": meta.id.name,
        "version": meta.id.version,
        "purl": purl,
        "hashes": hashes,
        "comment": artifact_key,
    });

    let dep_components: Vec<_> = deps
        .iter()
        .map(|d| {
            let dep_purl =
                registry_to_purl(&meta.id.registry, &d.name, d.version_req.as_deref().unwrap_or("*"));
            serde_json::json!({
                "type": "library",
                "name": d.name,
                "version": d.version_req.as_deref().unwrap_or(""),
                "purl": dep_purl,
            })
        })
        .collect();

    let mut components = vec![main_component];
    components.extend(dep_components);

    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.4",
        "version": 1,
        "serialNumber": format!("urn:uuid:{}", Uuid::new_v4()),
        "metadata": {
            "timestamp": Utc::now().to_rfc3339(),
            "component": {
                "type": "library",
                "name": meta.id.name,
                "version": meta.id.version,
            }
        },
        "components": components,
    })
}

// ── SbomService implementation ────────────────────────────────────────────────

impl SbomService {
    pub fn new(
        repo: Arc<dyn SbomRepository>,
        extractor: Option<Arc<dyn SbomExtractor>>,
        fetcher: Option<Arc<dyn UpstreamSbomFetcher>>,
    ) -> Self {
        Self { repo, extractor, fetcher }
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
                SbomFormat::Spdx => {
                    upstream_spdx
                        .clone()
                        .unwrap_or_else(|| generate_spdx(meta, artifact_key, &deps))
                }
                SbomFormat::CycloneDx => generate_cyclonedx(meta, artifact_key, &deps),
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
        let SbomPublishOptions { registry_type, formats, required } = opts;
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
                SbomFormat::Spdx => generate_spdx(&fake_meta, artifact_key, &deps),
                SbomFormat::CycloneDx => generate_cyclonedx(&fake_meta, artifact_key, &deps),
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
        let page_size: u64 = 100;
        let mut offset: u64 = 0;

        match format {
            SbomFormat::Spdx => {
                let mut all_packages: Vec<serde_json::Value> = Vec::new();
                let mut all_relationships: Vec<serde_json::Value> = Vec::new();
                let mut seen: HashSet<String> = HashSet::new();

                loop {
                    let page = self
                        .repo
                        .list_sboms_for_export(registry, from, to, page_size, offset)
                        .await?;
                    let done = page.len() < page_size as usize;
                    for sbom in page {
                        if let Some(pkgs) = sbom.document.get("packages").and_then(|v| v.as_array()) {
                            for pkg in pkgs {
                                let key = format!(
                                    "{}@{}",
                                    pkg.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                    pkg.get("versionInfo").and_then(|v| v.as_str()).unwrap_or(""),
                                );
                                if seen.insert(key) {
                                    all_packages.push(pkg.clone());
                                }
                            }
                        }
                        if let Some(rels) =
                            sbom.document.get("relationships").and_then(|v| v.as_array())
                        {
                            all_relationships.extend_from_slice(rels);
                        }
                    }
                    offset += page_size;
                    if done {
                        break;
                    }
                }

                Ok(serde_json::json!({
                    "spdxVersion": "SPDX-2.3",
                    "dataLicense": "CC0-1.0",
                    "SPDXID": "SPDXRef-DOCUMENT",
                    "name": format!("batlehub-org-export-{}", Utc::now().format("%Y%m%d")),
                    "documentNamespace": format!(
                        "https://batlehub/sbom/export/{}",
                        Uuid::new_v4()
                    ),
                    "packages": all_packages,
                    "relationships": all_relationships,
                }))
            }

            SbomFormat::CycloneDx => {
                let mut all_components: Vec<serde_json::Value> = Vec::new();
                let mut seen: HashSet<String> = HashSet::new();

                loop {
                    let page = self
                        .repo
                        .list_sboms_for_export(registry, from, to, page_size, offset)
                        .await?;
                    let done = page.len() < page_size as usize;
                    for sbom in page {
                        if let Some(comps) =
                            sbom.document.get("components").and_then(|v| v.as_array())
                        {
                            for comp in comps {
                                let key = format!(
                                    "{}@{}",
                                    comp.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                    comp.get("version").and_then(|v| v.as_str()).unwrap_or(""),
                                );
                                if seen.insert(key) {
                                    all_components.push(comp.clone());
                                }
                            }
                        }
                    }
                    offset += page_size;
                    if done {
                        break;
                    }
                }

                Ok(serde_json::json!({
                    "bomFormat": "CycloneDX",
                    "specVersion": "1.4",
                    "version": 1,
                    "serialNumber": format!("urn:uuid:{}", Uuid::new_v4()),
                    "metadata": {
                        "timestamp": Utc::now().to_rfc3339(),
                    },
                    "components": all_components,
                }))
            }
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::PackageId;

    fn make_meta(registry: &str, name: &str, version: &str, checksum: Option<&str>) -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new(registry, name, version),
            published_at: None,
            download_url: None,
            checksum: checksum.map(|s| s.to_owned()),
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        }
    }

    #[test]
    fn generate_spdx_required_fields() {
        let meta = make_meta("cargo", "tokio", "1.0.0", Some("abc123"));
        let doc = generate_spdx(&meta, "artifact:cargo/tokio/1.0.0", &[]);

        assert_eq!(doc["spdxVersion"], "SPDX-2.3");
        assert_eq!(doc["dataLicense"], "CC0-1.0");
        assert_eq!(doc["packages"][0]["versionInfo"], "1.0.0");
        assert_eq!(doc["packages"][0]["checksums"][0]["algorithm"], "SHA256");
        assert_eq!(doc["packages"][0]["checksums"][0]["checksumValue"], "abc123");
        assert_eq!(doc["relationships"][0]["relationshipType"], "DESCRIBES");
    }

    #[test]
    fn generate_spdx_no_checksum() {
        let meta = make_meta("npm", "lodash", "4.17.21", None);
        let doc = generate_spdx(&meta, "k", &[]);
        assert!(doc["packages"][0]["checksums"].as_array().unwrap().is_empty());
    }

    #[test]
    fn generate_spdx_with_deps() {
        let meta = make_meta("npm", "express", "4.0.0", None);
        let deps = vec![
            SbomDependency { name: "accepts".into(), version_req: Some("1.3.8".into()), ecosystem: "npm".into() },
        ];
        let doc = generate_spdx(&meta, "k", &deps);
        // main package + 1 dep
        assert_eq!(doc["packages"].as_array().unwrap().len(), 2);
        assert_eq!(doc["relationships"].as_array().unwrap().len(), 2);
        assert_eq!(doc["relationships"][1]["relationshipType"], "DEPENDENCY_OF");
    }

    #[test]
    fn generate_cyclonedx_required_fields() {
        let meta = make_meta("cargo", "serde", "1.0.0", Some("deadbeef"));
        let doc = generate_cyclonedx(&meta, "k", &[]);

        assert_eq!(doc["bomFormat"], "CycloneDX");
        assert_eq!(doc["specVersion"], "1.4");
        assert_eq!(doc["components"][0]["name"], "serde");
        assert_eq!(doc["components"][0]["version"], "1.0.0");
        assert_eq!(doc["components"][0]["purl"], "pkg:cargo/serde@1.0.0");
        assert_eq!(doc["components"][0]["hashes"][0]["alg"], "SHA-256");
    }

    #[test]
    fn registry_to_purl_variants() {
        assert_eq!(registry_to_purl("cargo", "tokio", "1.0.0"), "pkg:cargo/tokio@1.0.0");
        assert_eq!(registry_to_purl("npm", "lodash", "4.17.21"), "pkg:npm/lodash@4.17.21");
        assert_eq!(registry_to_purl("pypi", "requests", "2.31.0"), "pkg:pypi/requests@2.31.0");
        assert_eq!(registry_to_purl("rubygems", "rails", "7.0.0"), "pkg:gem/rails@7.0.0");
        assert_eq!(registry_to_purl("goproxy", "github.com/gin-gonic/gin", "v1.9.0"), "pkg:golang/github.com/gin-gonic/gin@v1.9.0");
        assert_eq!(registry_to_purl("unknown", "foo", "1.0"), "pkg:generic/foo@1.0");
    }
}
