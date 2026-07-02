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
use super::client::{is_release_signed, static_artifact_url, ForgejoRegistryClient};

// ── Serde types for Forgejo/Gitea API responses ──────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct FjRelease {
    pub id: u64,
    pub tag_name: String,
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<FjAsset>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FjAsset {
    pub id: u64,
    pub name: String,
    pub browser_download_url: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub size: u64,
}

impl ForgejoRegistryClient {
    /// Resolve the upstream download URL for a release asset identified either by
    /// filename (`filename/<name>`) or attachment id (numeric). Both look the asset
    /// up inside the release fetched by tag, so the caller must pass a real tag.
    async fn asset_download_url(
        &self,
        owner_repo: &str,
        tag: &str,
        artifact: &str,
    ) -> Result<String, CoreError> {
        let release = self.fetch_release_by_tag(owner_repo, tag).await?;
        if let Some(filename) = artifact.strip_prefix("filename/") {
            release
                .assets
                .iter()
                .find(|a| a.name == filename)
                .map(|a| a.browser_download_url.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!(
                        "no asset named '{filename}' in {owner_repo}@{tag}"
                    ))
                })
        } else {
            let asset_id: u64 = artifact
                .parse()
                .map_err(|_| CoreError::Registry(format!("invalid asset id: {artifact}")))?;
            release
                .assets
                .iter()
                .find(|a| a.id == asset_id)
                .map(|a| a.browser_download_url.clone())
                .ok_or_else(|| {
                    CoreError::NotFound(format!("asset {asset_id} not found in {owner_repo}@{tag}"))
                })
        }
    }
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for ForgejoRegistryClient {
    fn registry_type(&self) -> &str {
        "forgejo"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let owner_repo = &pkg.name;

        // Source archive / raw downloads and package-registry passthrough use no
        // release tag; return minimal metadata.
        if let Some(ref artifact) = pkg.artifact {
            if artifact.starts_with("raw/")
                || artifact.starts_with("tarball/")
                || artifact == "zipball"
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
                let releases = self.fetch_all_releases(owner_repo).await?;

                let extra = serde_json::to_value(releases.iter().map(|r| {
                    serde_json::json!({ "id": r.id, "tag_name": r.tag_name, "published_at": r.published_at })
                }).collect::<Vec<_>>()).unwrap_or_default();

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
                let release = self.fetch_release_by_tag(owner_repo, tag).await?;

                let published_at = release
                    .published_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let is_signed = is_release_signed(&release.assets);

                let download_url = match &pkg.artifact {
                    Some(artifact_str) => release
                        .assets
                        .iter()
                        .find(|a| {
                            artifact_str
                                .strip_prefix("filename/")
                                .map(|f| a.name == f)
                                .unwrap_or_else(|| artifact_str.parse::<u64>().ok() == Some(a.id))
                        })
                        .map(|a| a.browser_download_url.clone()),
                    None => None,
                };

                let extra = serde_json::json!({
                    "release_id": release.id,
                    "tag_name": release.tag_name,
                    "assets": release.assets.iter().map(|a| serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "download_url": a.browser_download_url,
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
        let owner_repo = &pkg.name;
        let git_ref = &pkg.version;

        let download_url = match &pkg.artifact {
            // Package-registry passthrough: `pkgpath/<instance-relative-path>` →
            // `{instance}/<path>` (e.g. `api/packages/{owner}/generic/…`).
            Some(artifact) if artifact.starts_with("pkgpath/") => {
                format!("{}/{}", self.base_url, &artifact["pkgpath/".len()..])
            }
            Some(artifact) => {
                if let Some(url) =
                    static_artifact_url(artifact, &self.base_url, owner_repo, git_ref)
                {
                    url
                } else {
                    self.asset_download_url(owner_repo, git_ref, artifact)
                        .await?
                }
            }
            None => {
                return Err(CoreError::Registry(
                    "fetch_artifact requires PackageId::artifact to be set".to_owned(),
                ));
            }
        };

        tracing::debug!(url = %download_url, "fetching Forgejo artifact");

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::forgejo::ForgejoRegistryClient;
    use crate::registry::http_client::UpstreamHttpOptions;

    #[tokio::test]
    async fn passthrough_artifacts_resolve_without_http() {
        // raw/tarball/zipball/pkgpath need no release lookup, so resolve_metadata
        // makes no HTTP call — an unreachable upstream proves it never connects.
        let c = ForgejoRegistryClient::new("http://127.0.0.1:1", &UpstreamHttpOptions::default())
            .unwrap();
        for art in [
            "raw/main/README.md",
            "tarball/v1.0.0",
            "zipball",
            "pkgpath/api/packages/owner/generic/x/1/file",
        ] {
            let pkg = PackageId::new("fj", "owner/repo", "v1.0.0").with_artifact(art);
            let md = c
                .resolve_metadata(&pkg)
                .await
                .expect("passthrough must not hit the network");
            assert_eq!(md.id.artifact.as_deref(), Some(art));
            assert!(md.download_url.is_none());
        }
        assert_eq!(c.registry_type(), "forgejo");
    }
}
