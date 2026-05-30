use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_tls, upstream_auth_headers, UpstreamHttpOptions};

/// GitHub REST API v3 registry client.
///
/// Supported `PackageId` conventions:
/// - `version = "releases"` → list releases (metadata only, no artifact)
/// - `version = "v1.80.0"` → release by tag (metadata for age-gate rule)
/// - `artifact = Some("12345678")` → specific release asset download (by ID)
/// - `artifact = Some("filename/{name}")` → release asset download (by filename)
/// - `artifact = Some("tarball/{ref}")` → source tarball (github.com/archive/)
/// - `artifact = Some("zipball")` → zip archive (github.com/archive/)
/// - `artifact = Some("raw/{path}")` → raw file (raw.githubusercontent.com)
pub struct GithubRegistryClient {
    http: reqwest::Client,
    base_url: String,
    /// Base URL for raw file downloads (default: `https://raw.githubusercontent.com`).
    raw_base_url: String,
    /// Base URL for archive downloads (default: `https://github.com`).
    archive_base_url: String,
    basic_auth: Option<(String, String)>,
}

impl GithubRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        // GitHub-specific default headers merged with any auth headers from opts.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            "application/vnd.github+json".parse().unwrap(),
        );
        headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
        let auth_headers = upstream_auth_headers(opts)?;
        headers.extend(auth_headers);

        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .default_headers(headers);
        let builder = apply_upstream_tls(builder, opts)?;
        let http = builder.build()?;

        let base_url = base_url.into();

        // Derive content URLs from the API base URL.
        // api.github.com → raw.githubusercontent.com / github.com
        // GitHub Enterprise → same host, no /api/v3 prefix
        let (raw_base_url, archive_base_url) = if base_url.contains("api.github.com") {
            (
                "https://raw.githubusercontent.com".to_owned(),
                "https://github.com".to_owned(),
            )
        } else {
            let host = base_url.trim_end_matches('/').trim_end_matches("/api/v3");
            (host.to_owned(), host.to_owned())
        };

        Ok(Self {
            http,
            base_url,
            raw_base_url,
            archive_base_url,
            basic_auth: opts.basic_auth.clone(),
        })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }
}

fn is_release_signed(assets: &[GhAsset]) -> bool {
    assets
        .iter()
        .any(|a| a.name.ends_with(".asc") || a.name.ends_with(".sig"))
}

/// Build a direct download URL for non-API artifact types (tarball, zipball, raw).
/// Returns `None` for asset-ID or filename-based artifact strings that need an API call.
fn static_artifact_url(
    artifact: &str,
    archive_base: &str,
    raw_base: &str,
    owner_repo: &str,
    git_ref: &str,
) -> Option<String> {
    if artifact.starts_with("tarball/") {
        Some(format!(
            "{}/{}/archive/{}.tar.gz",
            archive_base, owner_repo, git_ref
        ))
    } else if artifact == "zipball" {
        Some(format!(
            "{}/{}/archive/{}.zip",
            archive_base, owner_repo, git_ref
        ))
    } else {
        artifact
            .strip_prefix("raw/")
            .map(|file_path| format!("{}/{}/{}/{}", raw_base, owner_repo, git_ref, file_path))
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
                // List releases — return minimal metadata (no artifact URL).
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
                // Fetch release by tag to get published_at and asset list.
                let release = self.fetch_release_by_tag(owner_repo, tag).await?;

                let published_at = release
                    .published_at
                    .as_deref()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc));

                let is_signed = is_release_signed(&release.assets);

                // If an artifact was requested, resolve the download URL.
                let download_url = if let Some(artifact_str) = &pkg.artifact {
                    if let Some(filename) = artifact_str.strip_prefix("filename/") {
                        // Lookup by filename (used by the releases/download/{tag}/{file} route).
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
                // Resolve asset by filename against the release tag.
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
                // asset ID — first resolve the download URL via API
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: u64, name: &str) -> GhAsset {
        GhAsset {
            id,
            name: name.to_string(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 0,
        }
    }

    #[test]
    fn is_signed_true_when_asc_present() {
        let assets = vec![asset(1, "binary.tar.gz"), asset(2, "binary.tar.gz.asc")];
        assert!(is_release_signed(&assets));
    }

    #[test]
    fn is_signed_true_when_sig_present() {
        let assets = vec![asset(1, "binary.zip"), asset(2, "binary.zip.sig")];
        assert!(is_release_signed(&assets));
    }

    #[test]
    fn is_signed_false_when_no_sig_asset() {
        let assets = vec![asset(1, "binary.tar.gz"), asset(2, "checksums.txt")];
        assert!(!is_release_signed(&assets));
    }

    #[test]
    fn static_url_tarball() {
        let url = static_artifact_url(
            "tarball/main",
            "https://github.com",
            "https://raw.githubusercontent.com",
            "owner/repo",
            "v1.0",
        );
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/owner/repo/archive/v1.0.tar.gz")
        );
    }

    #[test]
    fn static_url_zipball() {
        let url = static_artifact_url(
            "zipball",
            "https://github.com",
            "https://raw.githubusercontent.com",
            "owner/repo",
            "v1.0",
        );
        assert_eq!(
            url.as_deref(),
            Some("https://github.com/owner/repo/archive/v1.0.zip")
        );
    }

    #[test]
    fn static_url_raw_file() {
        let url = static_artifact_url(
            "raw/src/main.rs",
            "https://github.com",
            "https://raw.githubusercontent.com",
            "owner/repo",
            "main",
        );
        assert_eq!(
            url.as_deref(),
            Some("https://raw.githubusercontent.com/owner/repo/main/src/main.rs")
        );
    }

    #[test]
    fn static_url_none_for_asset_id() {
        let url = static_artifact_url(
            "12345678",
            "https://github.com",
            "https://raw.githubusercontent.com",
            "owner/repo",
            "v1.0",
        );
        assert!(url.is_none());
    }
}

impl GithubRegistryClient {
    async fn fetch_release_by_tag(
        &self,
        owner_repo: &str,
        tag: &str,
    ) -> Result<GhRelease, CoreError> {
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.base_url, owner_repo, tag
        );
        let resp = self
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
