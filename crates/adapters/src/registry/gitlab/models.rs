use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::super::http_client::to_registry_error;
use super::client::{is_release_signed, source_format, GitlabRegistryClient};

// ── Serde types for GitLab API responses ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GlRelease {
    pub tag_name: String,
    /// GitLab uses `released_at` (not `published_at`).
    pub released_at: Option<String>,
    #[serde(default)]
    pub assets: GlAssets,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GlAssets {
    #[serde(default)]
    pub links: Vec<GlLink>,
    #[serde(default)]
    pub sources: Vec<GlSource>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GlLink {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub direct_asset_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GlSource {
    pub format: String,
    pub url: String,
}

impl GitlabRegistryClient {
    /// Resolve a release link asset to its upstream download URL, matched by the
    /// link `name`.
    async fn link_download_url(
        &self,
        project: &str,
        tag: &str,
        name: &str,
    ) -> Result<String, CoreError> {
        let release = self.fetch_release_by_tag(project, tag).await?;
        release
            .assets
            .links
            .iter()
            .find(|l| l.name == name)
            // Prefer the direct asset URL (stable permalink) when present.
            .map(|l| l.direct_asset_url.clone().unwrap_or_else(|| l.url.clone()))
            .ok_or_else(|| {
                CoreError::NotFound(format!("no release link named '{name}' in {project}@{tag}"))
            })
    }
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for GitlabRegistryClient {
    fn registry_type(&self) -> &str {
        "gitlab"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let project = &pkg.name;

        // Source archives, raw files, and package-registry passthrough need no
        // release lookup — return minimal metadata.
        if let Some(ref artifact) = pkg.artifact {
            if source_format(artifact).is_some()
                || artifact.starts_with("rawfile/")
                || artifact.starts_with("pkgpath/")
            {
                return Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control: None,
                });
            }
        }

        match pkg.version.as_str() {
            "releases" => {
                let releases = self.fetch_all_releases(project).await?;

                let extra = serde_json::to_value(
                    releases
                        .iter()
                        .map(|r| {
                            serde_json::json!({ "tag_name": r.tag_name, "released_at": r.released_at })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default();

                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra,
                    cache_control: None,
                })
            }

            tag => {
                let release = self.fetch_release_by_tag(project, tag).await?;

                let published_at = release
                    .released_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let is_signed = is_release_signed(&release.assets.links);

                let download_url = match &pkg.artifact {
                    Some(artifact) => artifact.strip_prefix("link/").and_then(|name| {
                        release
                            .assets
                            .links
                            .iter()
                            .find(|l| l.name == name)
                            .map(|l| l.direct_asset_url.clone().unwrap_or_else(|| l.url.clone()))
                    }),
                    None => None,
                };

                let extra = serde_json::json!({
                    "tag_name": release.tag_name,
                    "links": release.assets.links.iter().map(|l| serde_json::json!({
                        "name": l.name,
                        "url": l.url,
                    })).collect::<Vec<_>>(),
                    "sources": release.assets.sources.iter().map(|s| serde_json::json!({
                        "format": s.format,
                        "url": s.url,
                    })).collect::<Vec<_>>(),
                });

                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at,
                    download_url,
                    checksum: None,
                    is_signed: Some(is_signed),
                    extra,
                    cache_control: None,
                })
            }
        }
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let project = &pkg.name;
        let tag = &pkg.version;

        let download_url = match &pkg.artifact {
            Some(artifact) => {
                if let Some(format) = source_format(artifact) {
                    self.source_archive_url(project, tag, format)
                } else if let Some(name) = artifact.strip_prefix("link/") {
                    self.link_download_url(project, tag, name).await?
                } else if let Some(rest) = artifact.strip_prefix("rawfile/") {
                    // `rawfile/{ref}/{path}` → repository raw-file API.
                    let (git_ref, path) = rest.split_once('/').ok_or_else(|| {
                        CoreError::Registry(format!("invalid rawfile selector: {artifact}"))
                    })?;
                    self.raw_file_url(project, git_ref, path)
                } else if let Some(rest) = artifact.strip_prefix("pkgpath/") {
                    // Package-registry passthrough (`api/v4/projects/.../packages/…`).
                    self.passthrough_url(rest)
                } else {
                    return Err(CoreError::Registry(format!(
                        "unsupported gitlab artifact selector: {artifact}"
                    )));
                }
            }
            None => {
                return Err(CoreError::Registry(
                    "fetch_artifact requires PackageId::artifact to be set".to_owned(),
                ));
            }
        };

        tracing::debug!(url = %download_url, "fetching GitLab artifact");

        let response = self
            .get(&download_url)
            .send()
            .await
            .map_err(to_registry_error)?
            .error_for_status()
            .map_err(to_registry_error)?;

        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = response.bytes_stream().map_err(to_registry_error);

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        match self.fetch_all_releases(package).await {
            Ok(releases) => Ok(releases.into_iter().map(|r| r.tag_name).collect()),
            Err(CoreError::NotFound(_)) => Ok(vec![]),
            Err(e) => Err(e),
        }
    }
}
