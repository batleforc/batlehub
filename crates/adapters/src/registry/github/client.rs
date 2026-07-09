use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;

use super::super::http_client::{
    apply_upstream_tls, basic_auth_get, to_registry_error, upstream_auth_headers,
    UpstreamHttpOptions,
};
use super::models::{GhAsset, GhRelease};
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

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
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    /// Base URL for raw file downloads (default: `https://raw.githubusercontent.com`).
    pub(super) raw_base_url: String,
    /// Base URL for archive downloads (default: `https://github.com`).
    pub(super) archive_base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
}

impl GithubRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> Result<Self, CoreError> {
        // GitHub-specific default headers merged with any auth headers from opts.
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            "application/vnd.github+json"
                .parse()
                .expect("static header value is valid ASCII"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            "2022-11-28"
                .parse()
                .expect("static header value is valid ASCII"),
        );
        let auth_headers = upstream_auth_headers(opts)?;
        headers.extend(auth_headers);

        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .default_headers(headers);
        let builder = apply_upstream_tls(builder, opts)?;
        let http = builder.build().map_err(|e| CoreError::Other(e.into()))?;

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

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    pub(super) async fn fetch_release_by_tag(
        &self,
        owner_repo: &str,
        tag: &str,
    ) -> Result<GhRelease, CoreError> {
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.base_url, owner_repo, tag
        );
        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{owner_repo}@{tag} not found")));
        }

        resp.error_for_status()
            .map_err(to_registry_error)?
            .json::<GhRelease>()
            .await
            .map_err(to_registry_error)
    }
}

// ── Pure helper functions (also used by models.rs RegistryClient impl) ────────

pub(super) fn is_release_signed(assets: &[GhAsset]) -> bool {
    assets
        .iter()
        .any(|a| a.name.ends_with(".asc") || a.name.ends_with(".sig"))
}

/// Build a direct download URL for non-API artifact types (tarball, zipball, raw).
pub(super) fn static_artifact_url(
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

/// Parse the `Link` header and return the URL for `rel="next"`, if present.
pub(super) fn next_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in link.split(',') {
        let mut url_part = None;
        let mut is_next = false;
        for segment in part.split(';') {
            let s = segment.trim();
            if s.starts_with('<') && s.ends_with('>') {
                url_part = Some(s[1..s.len() - 1].to_owned());
            } else if s == r#"rel="next""# {
                is_next = true;
            }
        }
        if is_next {
            return url_part;
        }
    }
    None
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
                return Ok(PackageMetadata::minimal(
                    pkg.clone(),
                    serde_json::Value::Null,
                ));
            }
        }

        match pkg.version.as_str() {
            "releases" => {
                let url = format!("{}/repos/{}/releases", self.base_url, owner_repo);
                let resp = self.get(&url).send().await.map_err(to_registry_error)?;

                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(CoreError::NotFound(format!("{owner_repo} not found")));
                }

                let releases: Vec<GhRelease> = resp
                    .error_for_status()
                    .map_err(to_registry_error)?
                    .json()
                    .await
                    .map_err(to_registry_error)?;

                let extra = serde_json::to_value(releases.iter().map(|r| {
                    serde_json::json!({ "id": r.id, "tag_name": r.tag_name, "published_at": r.published_at })
                }).collect::<Vec<_>>()).unwrap_or_default();

                Ok(PackageMetadata::minimal(pkg.clone(), extra))
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
                let resp = self.get(&url).send().await.map_err(to_registry_error)?;

                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Err(CoreError::NotFound(format!("asset {asset_id} not found")));
                }

                let asset: GhAsset = resp
                    .error_for_status()
                    .map_err(to_registry_error)?
                    .json()
                    .await
                    .map_err(to_registry_error)?;
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
        let mut url = format!("{}/repos/{}/releases?per_page=100", self.base_url, package);
        let mut versions = Vec::new();

        for _ in 0..10 {
            let resp = self.get(&url).send().await.map_err(to_registry_error)?;

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

            let releases: Vec<GhRelease> = resp.json().await.map_err(to_registry_error)?;

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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::ports::RegistryClient;

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

    #[test]
    fn next_link_parses_rel_next() {
        let mut map = reqwest::header::HeaderMap::new();
        map.insert(
            reqwest::header::LINK,
            r#"<https://api.github.com/repos/owner/repo/releases?page=2&per_page=100>; rel="next", <https://api.github.com/repos/owner/repo/releases?page=5&per_page=100>; rel="last""#
                .parse()
                .unwrap(),
        );
        assert_eq!(
            next_link(&map).as_deref(),
            Some("https://api.github.com/repos/owner/repo/releases?page=2&per_page=100")
        );
    }

    #[test]
    fn next_link_absent_when_no_next_rel() {
        let mut map = reqwest::header::HeaderMap::new();
        map.insert(
            reqwest::header::LINK,
            r#"<https://api.github.com/repos/owner/repo/releases?page=5&per_page=100>; rel="last""#
                .parse()
                .unwrap(),
        );
        assert!(next_link(&map).is_none());
    }

    #[tokio::test]
    async fn list_versions_single_page() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!([
            { "id": 1, "tag_name": "v1.1.0", "published_at": "2024-01-02T00:00:00Z", "assets": [] },
            { "id": 2, "tag_name": "v1.0.0", "published_at": "2024-01-01T00:00:00Z", "assets": [] },
        ]))
        .unwrap();
        let _mock = server
            .mock("GET", "/repos/owner/repo/releases?per_page=100")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = GithubRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("owner/repo").await.unwrap();
        assert_eq!(versions, vec!["v1.1.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn list_versions_follows_pagination() {
        let mut server = mockito::Server::new_async().await;

        let page1 = serde_json::to_string(&serde_json::json!([
            { "id": 1, "tag_name": "v1.2.0", "published_at": null, "assets": [] }
        ]))
        .unwrap();
        let page2 = serde_json::to_string(&serde_json::json!([
            { "id": 2, "tag_name": "v1.1.0", "published_at": null, "assets": [] },
            { "id": 3, "tag_name": "v1.0.0", "published_at": null, "assets": [] }
        ]))
        .unwrap();

        let page2_url = format!(
            "{}/repos/owner/repo/releases?page=2&per_page=100",
            server.url()
        );
        let link_header = format!(r#"<{page2_url}>; rel="next""#);

        let _m1 = server
            .mock("GET", "/repos/owner/repo/releases?per_page=100")
            .with_status(200)
            .with_header("link", &link_header)
            .with_body(&page1)
            .create_async()
            .await;
        let _m2 = server
            .mock("GET", "/repos/owner/repo/releases?page=2&per_page=100")
            .with_status(200)
            .with_body(&page2)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = GithubRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("owner/repo").await.unwrap();
        assert_eq!(versions, vec!["v1.2.0", "v1.1.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn list_versions_404_returns_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/repos/unknown/repo/releases?per_page=100")
            .with_status(404)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = GithubRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("unknown/repo").await.unwrap();
        assert!(versions.is_empty());
    }
}
