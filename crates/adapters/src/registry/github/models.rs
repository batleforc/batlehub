use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::client::{is_release_signed, next_link, static_artifact_url, GithubRegistryClient};

// ── Serde types for GitHub API responses ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GhRelease {
    pub id: u64,
    pub tag_name: String,
    pub published_at: Option<String>,
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GhAsset {
    pub id: u64,
    pub name: String,
    pub browser_download_url: String,
    #[allow(dead_code)]
    pub size: u64,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for GithubRegistryClient {
    fn registry_type(&self) -> &str {
        "github"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let owner_repo = &pkg.name;

        // Raw file and archive downloads use a branch/SHA as the version, not a
        // release tag. Skip the releases API entirely and return minimal metadata.
        if let Some(ref artifact) = pkg.artifact {
            if artifact.starts_with("raw/")
                || artifact.starts_with("tarball/")
                || artifact == "zipball"
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
                let url = format!("{}/repos/{}/releases", self.base_url, owner_repo);
                let resp = self
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| CoreError::Registry(e.to_string()))?;

                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(CoreError::NotFound(format!("{owner_repo} not found")));
                }

                let releases: Vec<GhRelease> = resp
                    .error_for_status()
                    .map_err(|e| CoreError::Registry(e.to_string()))?
                    .json()
                    .await
                    .map_err(|e| CoreError::Registry(e.to_string()))?;

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

                let download_url = if let Some(artifact_str) = &pkg.artifact {
                    if let Some(filename) = artifact_str.strip_prefix("filename/") {
                        release
                            .assets
                            .iter()
                            .find(|a| a.name == filename)
                            .map(|a| a.browser_download_url.clone())
                    } else {
                        let asset_id: u64 = artifact_str.parse().map_err(|_| {
                            CoreError::Registry(format!("invalid asset id: {artifact_str}"))
                        })?;
                        release
                            .assets
                            .iter()
                            .find(|a| a.id == asset_id)
                            .map(|a| a.browser_download_url.clone())
                    }
                } else {
                    None
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

        let download_url = if let Some(artifact) = &pkg.artifact {
            if let Some(url) = static_artifact_url(
                artifact,
                &self.archive_base_url,
                &self.raw_base_url,
                owner_repo,
                git_ref,
            ) {
                url
            } else if let Some(filename) = artifact.strip_prefix("filename/") {
                let release = self.fetch_release_by_tag(owner_repo, git_ref).await?;
                release
                    .assets
                    .iter()
                    .find(|a| a.name == filename)
                    .map(|a| a.browser_download_url.clone())
                    .ok_or_else(|| {
                        CoreError::NotFound(format!(
                            "no asset named '{filename}' in {owner_repo}@{git_ref}"
                        ))
                    })?
            } else {
                let asset_id: u64 = artifact
                    .parse()
                    .map_err(|_| CoreError::Registry(format!("invalid asset id: {artifact}")))?;
                let url = format!(
                    "{}/repos/{}/releases/assets/{}",
                    self.base_url, owner_repo, asset_id
                );
                let resp = self
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| CoreError::Registry(e.to_string()))?;

                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(CoreError::NotFound(format!("asset {asset_id} not found")));
                }

                let asset: GhAsset = resp
                    .error_for_status()
                    .map_err(|e| CoreError::Registry(e.to_string()))?
                    .json()
                    .await
                    .map_err(|e| CoreError::Registry(e.to_string()))?;
                asset.browser_download_url
            }
        } else {
            return Err(CoreError::Registry(
                "fetch_artifact requires PackageId::artifact to be set".to_owned(),
            ));
        };

        tracing::debug!(url = %download_url, "fetching GitHub artifact");

        let response = self
            .get(&download_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = response
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let mut url = format!("{}/repos/{}/releases?per_page=100", self.base_url, package);
        let mut versions = Vec::new();

        for _ in 0..10 {
            let resp = self
                .get(&url)
                .send()
                .await
                .map_err(|e| CoreError::Registry(e.to_string()))?;

            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(vec![]);
            }
            if !resp.status().is_success() {
                return Err(CoreError::Registry(format!(
                    "github: releases list returned {}",
                    resp.status()
                )));
            }

            let next_url = next_link(resp.headers());

            let releases: Vec<GhRelease> = resp
                .json()
                .await
                .map_err(|e| CoreError::Registry(e.to_string()))?;

            for r in releases {
                versions.push(r.tag_name);
            }

            match next_url {
                Some(next) => url = next,
                None => break,
            }
        }

        Ok(versions)
    }
}
