use super::super::http_client::{
    apply_upstream_tls, basic_auth_get, upstream_auth_headers, UpstreamHttpOptions,
};
use super::models::{FjAsset, FjRelease};
use batlehub_core::error::CoreError;

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
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
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
        let http = builder.build()?;

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

    pub(super) async fn fetch_release_by_tag(
        &self,
        owner_repo: &str,
        tag: &str,
    ) -> Result<FjRelease, CoreError> {
        let url = format!(
            "{}/repos/{}/releases/tags/{}",
            self.api_base_url, owner_repo, tag
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
            .json::<FjRelease>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
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
