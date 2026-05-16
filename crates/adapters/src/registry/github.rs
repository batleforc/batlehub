use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use proxy_cache_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, RegistryClient},
};

/// GitHub REST API v3 registry client.
///
/// Supported `PackageId` conventions:
/// - `version = "releases"` → list releases (metadata only, no artifact)
/// - `version = "v1.80.0"` → release by tag (metadata for age-gate rule)
/// - `artifact = Some("12345678")` → specific release asset download
/// - `artifact = Some("tarball/v1.80.0")` → source tarball download
pub struct GithubRegistryClient {
    http: reqwest::Client,
    base_url: String,
}

impl GithubRegistryClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            "application/vnd.github+json".parse().unwrap(),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            "2022-11-28".parse().unwrap(),
        );
        if let Some(tok) = token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {tok}").parse().unwrap(),
            );
        }
        let http = reqwest::Client::builder()
            .user_agent("proxy-cache/0.1")
            .default_headers(headers)
            .build()
            .expect("failed to build GitHub HTTP client");

        Self {
            http,
            base_url: base_url.into(),
        }
    }
}

// ── Serde types for GitHub API responses ─────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GhRelease {
    id: u64,
    tag_name: String,
    published_at: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    id: u64,
    name: String,
    browser_download_url: String,
    #[allow(dead_code)]
    size: u64,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for GithubRegistryClient {
    fn registry_type(&self) -> &str {
        "github"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        // `name` is expected to be "owner/repo".
        let owner_repo = &pkg.name;

        match pkg.version.as_str() {
            "releases" => {
                // List releases — return minimal metadata (no artifact URL).
                let url = format!("{}/repos/{}/releases", self.base_url, owner_repo);
                let resp = self
                    .http
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

                let extra = serde_json::to_value(&releases.iter().map(|r| {
                    serde_json::json!({ "id": r.id, "tag_name": r.tag_name, "published_at": r.published_at })
                }).collect::<Vec<_>>()).unwrap_or_default();

                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra,
                })
            }

            tag => {
                // Fetch release by tag to get published_at and asset list.
                let release = self.fetch_release_by_tag(owner_repo, tag).await?;

                let published_at = release
                    .published_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                // Check for a .asc or .sig asset (detached GPG signature).
                let asset_names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
                let is_signed = asset_names.iter().any(|n| n.ends_with(".asc") || n.ends_with(".sig"));

                // If an artifact ID was requested, find the download URL.
                let download_url = if let Some(asset_id_str) = &pkg.artifact {
                    let asset_id: u64 = asset_id_str
                        .parse()
                        .map_err(|_| CoreError::Registry(format!("invalid asset id: {asset_id_str}")))?;
                    release
                        .assets
                        .iter()
                        .find(|a| a.id == asset_id)
                        .map(|a| a.browser_download_url.clone())
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
                })
            }
        }
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
        let owner_repo = &pkg.name;

        let download_url = if let Some(artifact) = &pkg.artifact {
            if artifact.starts_with("tarball/") {
                let r#ref = artifact.strip_prefix("tarball/").unwrap();
                format!("{}/repos/{}/tarball/{}", self.base_url, owner_repo, r#ref)
            } else {
                // asset ID — first resolve the download URL via API
                let asset_id: u64 = artifact
                    .parse()
                    .map_err(|_| CoreError::Registry(format!("invalid asset id: {artifact}")))?;
                let url = format!(
                    "{}/repos/{}/releases/assets/{}",
                    self.base_url, owner_repo, asset_id
                );
                let resp = self
                    .http
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
            .http
            .get(&download_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let stream = response
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(Box::pin(stream))
    }
}

impl GithubRegistryClient {
    async fn fetch_release_by_tag(&self, owner_repo: &str, tag: &str) -> Result<GhRelease, CoreError> {
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.base_url, owner_repo, tag
        );
        let resp = self.http
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{owner_repo}@{tag} not found")));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<GhRelease>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
    }
}
