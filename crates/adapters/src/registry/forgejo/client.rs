use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;

use super::super::http_client::{
    apply_upstream_tls, basic_auth_get, to_registry_error, upstream_auth_headers,
    UpstreamHttpOptions,
};
use super::models::{FjAsset, FjRelease};
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

/// Forgejo / Gitea REST API v1 registry client.
///
/// A single adapter serves both Forgejo and Gitea instances — the release API
/// (`/api/v1/repos/{owner}/{repo}/releases`) is identical between them. There is
/// no public default instance, so an upstream URL is required in config; the URL
/// is the instance root (e.g. `https://codeberg.org`), not the API path.
///
/// Supported `PackageId` conventions (mirrors the GitHub adapter):
/// - `version = "releases"` → list releases (metadata only, no artifact)
/// - `version = "v1.0.0"` → release by tag (metadata for age-gate rule)
/// - `artifact = Some("12345678")` → release asset download (by attachment ID)
/// - `artifact = Some("filename/{name}")` → release asset download (by filename)
/// - `artifact = Some("tarball/{ref}")` → source tarball (`/archive/{ref}.tar.gz`)
/// - `artifact = Some("zipball")` → zip archive (`/archive/{ref}.zip`)
/// - `artifact = Some("raw/{path}")` → raw file (`/raw/{ref}/{path}`)
pub struct ForgejoRegistryClient {
    pub(super) http: reqwest::Client,
    /// Instance root, e.g. `https://codeberg.org` (no trailing slash).
    pub(super) base_url: String,
    /// API base, derived as `{base_url}/api/v1`.
    pub(super) api_base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
}

impl ForgejoRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> Result<Self, CoreError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            "application/json"
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

        // The configured URL is the instance root. Strip a trailing `/api/v1`
        // (if a user pasted the API URL) and any trailing slash, then derive the
        // API base from the root.
        let root = base_url.into();
        let root = root.trim_end_matches('/');
        let root = root.trim_end_matches("/api/v1");
        let base_url = root.trim_end_matches('/').to_owned();
        let api_base_url = format!("{base_url}/api/v1");

        Ok(Self {
            http,
            base_url,
            api_base_url,
            basic_auth: opts.basic_auth.clone(),
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    /// Fetch every release for `owner_repo`, following `Link: rel="next"`
    /// pagination (Gitea/Forgejo default 50 per page; capped at 20 pages). Returns
    /// `NotFound` if the repository's releases endpoint 404s on the first page.
    pub(super) async fn fetch_all_releases(
        &self,
        owner_repo: &str,
    ) -> Result<Vec<FjRelease>, CoreError> {
        use super::super::http_client::next_link;
        let mut url = format!(
            "{}/repos/{}/releases?limit=50",
            self.api_base_url, owner_repo
        );
        let mut all = Vec::new();
        for page in 0..20 {
            let resp = self.get(&url).send().await.map_err(to_registry_error)?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                if page == 0 {
                    return Err(CoreError::NotFound(format!("{owner_repo} not found")));
                }
                break;
            }
            if !resp.status().is_success() {
                return Err(CoreError::Registry(format!(
                    "forgejo: releases list returned {}",
                    resp.status()
                )));
            }
            let next = next_link(resp.headers());
            let releases: Vec<FjRelease> = resp.json().await.map_err(to_registry_error)?;
            all.extend(releases);
            match next {
                Some(n) => url = n,
                None => break,
            }
        }
        Ok(all)
    }

    pub(super) async fn fetch_release_by_tag(
        &self,
        owner_repo: &str,
        tag: &str,
    ) -> Result<FjRelease, CoreError> {
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.api_base_url, owner_repo, tag
        );
        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{owner_repo}@{tag} not found")));
        }

        resp.error_for_status()
            .map_err(to_registry_error)?
            .json::<FjRelease>()
            .await
            .map_err(to_registry_error)
    }
}

// ── Pure helper functions (also used by models.rs RegistryClient impl) ────────

pub(super) fn is_release_signed(assets: &[FjAsset]) -> bool {
    assets
        .iter()
        .any(|a| a.name.ends_with(".asc") || a.name.ends_with(".sig"))
}

/// Build a direct download URL for non-API artifact types (tarball, zipball, raw).
///
/// Forgejo/Gitea serve these from the instance root, mirroring GitHub's layout:
/// `{base}/{owner}/{repo}/archive/{ref}.tar.gz` and `{base}/{owner}/{repo}/raw/{ref}/{path}`.
pub(super) fn static_artifact_url(
    artifact: &str,
    base: &str,
    owner_repo: &str,
    git_ref: &str,
) -> Option<String> {
    if artifact.starts_with("tarball/") {
        Some(format!("{base}/{owner_repo}/archive/{git_ref}.tar.gz"))
    } else if artifact == "zipball" {
        Some(format!("{base}/{owner_repo}/archive/{git_ref}.zip"))
    } else {
        artifact
            .strip_prefix("raw/")
            .map(|file_path| format!("{base}/{owner_repo}/raw/{git_ref}/{file_path}"))
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
                return Ok(PackageMetadata::minimal(
                    pkg.clone(),
                    serde_json::Value::Null,
                ));
            }
        }

        match pkg.version.as_str() {
            "releases" => {
                let releases = self.fetch_all_releases(owner_repo).await?;

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
    use batlehub_core::ports::RegistryClient;

    fn asset(id: u64, name: &str) -> FjAsset {
        FjAsset {
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
    fn is_signed_false_when_no_sig_asset() {
        let assets = vec![asset(1, "binary.tar.gz"), asset(2, "checksums.txt")];
        assert!(!is_release_signed(&assets));
    }

    #[test]
    fn static_url_tarball() {
        let url = static_artifact_url("tarball/main", "https://codeberg.org", "owner/repo", "v1.0");
        assert_eq!(
            url.as_deref(),
            Some("https://codeberg.org/owner/repo/archive/v1.0.tar.gz")
        );
    }

    #[test]
    fn static_url_raw_file() {
        let url = static_artifact_url(
            "raw/src/main.rs",
            "https://codeberg.org",
            "owner/repo",
            "main",
        );
        assert_eq!(
            url.as_deref(),
            Some("https://codeberg.org/owner/repo/raw/main/src/main.rs")
        );
    }

    #[test]
    fn new_derives_api_base_from_root() {
        let opts = UpstreamHttpOptions::default();
        let client = ForgejoRegistryClient::new("https://codeberg.org/", &opts).unwrap();
        assert_eq!(client.base_url, "https://codeberg.org");
        assert_eq!(client.api_base_url, "https://codeberg.org/api/v1");
    }

    #[test]
    fn new_strips_accidental_api_suffix() {
        let opts = UpstreamHttpOptions::default();
        let client = ForgejoRegistryClient::new("https://git.example.com/api/v1", &opts).unwrap();
        assert_eq!(client.base_url, "https://git.example.com");
        assert_eq!(client.api_base_url, "https://git.example.com/api/v1");
    }

    #[tokio::test]
    async fn list_versions_returns_tags() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!([
            { "id": 1, "tag_name": "v1.1.0", "published_at": "2024-01-02T00:00:00Z", "assets": [] },
            { "id": 2, "tag_name": "v1.0.0", "published_at": "2024-01-01T00:00:00Z", "assets": [] },
        ]))
        .unwrap();
        let _mock = server
            .mock("GET", "/api/v1/repos/owner/repo/releases?limit=50")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = ForgejoRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("owner/repo").await.unwrap();
        assert_eq!(versions, vec!["v1.1.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn list_versions_follows_pagination() {
        let mut server = mockito::Server::new_async().await;
        let page2_url = format!("{}/api/v1/repos/o/r/releases?limit=50&page=2", server.url());
        let _p1 = server
            .mock("GET", "/api/v1/repos/o/r/releases?limit=50")
            .with_status(200)
            .with_header("link", &format!(r#"<{page2_url}>; rel="next""#))
            .with_body(r#"[{"id":1,"tag_name":"v2.0.0","assets":[]}]"#)
            .create_async()
            .await;
        let _p2 = server
            .mock("GET", "/api/v1/repos/o/r/releases?limit=50&page=2")
            .with_status(200)
            .with_body(r#"[{"id":2,"tag_name":"v1.0.0","assets":[]}]"#)
            .create_async()
            .await;
        let client =
            ForgejoRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let versions = client.list_versions("o/r").await.unwrap();
        assert_eq!(versions, vec!["v2.0.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn fetch_artifact_package_passthrough() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/api/packages/acme/generic/tool/1.0/tool.bin")
            .with_status(200)
            .with_body(b"BINARY")
            .create_async()
            .await;
        let client =
            ForgejoRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("fj", "_packages", "_")
            .with_artifact("pkgpath/api/packages/acme/generic/tool/1.0/tool.bin");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let body =
            futures::TryStreamExt::try_fold(fetched.stream, Vec::new(), |mut a, c| async move {
                a.extend_from_slice(&c);
                Ok(a)
            })
            .await
            .unwrap();
        assert_eq!(body, b"BINARY");
    }

    #[tokio::test]
    async fn list_versions_404_returns_empty() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/v1/repos/unknown/repo/releases?limit=50")
            .with_status(404)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = ForgejoRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("unknown/repo").await.unwrap();
        assert!(versions.is_empty());
    }

    #[tokio::test]
    async fn resolve_metadata_releases_list_collects_tags() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!([
            { "id": 2, "tag_name": "v2.0.0", "published_at": null, "assets": [] },
            { "id": 1, "tag_name": "v1.0.0", "published_at": null, "assets": [] },
        ]))
        .unwrap();
        let _m = server
            .mock("GET", "/api/v1/repos/o/r/releases?limit=50")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;
        let client =
            ForgejoRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("fj", "o/r", "releases");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        let tags: Vec<_> = meta
            .extra
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["tag_name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(tags, vec!["v2.0.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn fetch_artifact_static_tarball_streams_bytes() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock("GET", "/o/r/archive/v1.0.0.tar.gz")
            .with_status(200)
            .with_body(b"TARBALL")
            .create_async()
            .await;
        let client =
            ForgejoRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("fj", "o/r", "v1.0.0")
            .with_artifact("tarball/v1.0.0");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let body =
            futures::TryStreamExt::try_fold(fetched.stream, Vec::new(), |mut a, c| async move {
                a.extend_from_slice(&c);
                Ok(a)
            })
            .await
            .unwrap();
        assert_eq!(body, b"TARBALL");
    }

    #[tokio::test]
    async fn fetch_artifact_by_filename_resolves_asset() {
        let mut server = mockito::Server::new_async().await;
        let dl = format!("{}/dl/app.bin", server.url());
        let rel = serde_json::to_string(&serde_json::json!({
            "id": 1, "tag_name": "v1", "published_at": null,
            "assets": [ { "id": 9, "name": "app.bin", "browser_download_url": dl, "size": 3 } ]
        }))
        .unwrap();
        let _m1 = server
            .mock("GET", "/api/v1/repos/o/r/releases/tags/v1")
            .with_status(200)
            .with_body(&rel)
            .create_async()
            .await;
        let _m2 = server
            .mock("GET", "/dl/app.bin")
            .with_status(200)
            .with_body(b"BIN")
            .create_async()
            .await;
        let client =
            ForgejoRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("fj", "o/r", "v1")
            .with_artifact("filename/app.bin");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let body =
            futures::TryStreamExt::try_fold(fetched.stream, Vec::new(), |mut a, c| async move {
                a.extend_from_slice(&c);
                Ok(a)
            })
            .await
            .unwrap();
        assert_eq!(body, b"BIN");
    }

    #[tokio::test]
    async fn resolve_metadata_by_tag_parses_assets() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!({
            "id": 7,
            "tag_name": "v2.0.0",
            "published_at": "2024-05-01T00:00:00Z",
            "assets": [
                { "id": 11, "name": "app.tar.gz", "browser_download_url": "https://dl/app.tar.gz", "size": 10 },
                { "id": 12, "name": "app.tar.gz.asc", "browser_download_url": "https://dl/app.tar.gz.asc", "size": 1 },
            ]
        }))
        .unwrap();
        let _mock = server
            .mock("GET", "/api/v1/repos/owner/repo/releases/tags/v2.0.0")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = ForgejoRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("fj", "owner/repo", "v2.0.0");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.is_signed, Some(true));
        assert!(meta.published_at.is_some());
    }
}
