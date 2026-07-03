use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;

use super::super::http_client::{
    apply_upstream_tls, basic_auth_get, percent_encode, to_registry_error, upstream_auth_headers,
    UpstreamHttpOptions,
};
use super::models::{GlLink, GlRelease};
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

/// GitLab REST API v4 registry client (releases).
///
/// `PackageId` conventions:
/// - `name = "{group}/{subgroup}/{project}"` — the full project path; URL-encoded
///   (`/` → `%2F`) before being used in the API project selector.
/// - `version = "releases"` → list releases (metadata only)
/// - `version = "v1.0.0"` → release by tag
/// - `artifact = Some("link/{name}")` → release link asset (matched by link name)
/// - `artifact = Some("source/{format}")` → source archive via the repository
///   archive endpoint (`format` ∈ `tar.gz`, `zip`, `tar.bz2`, `tar`)
///
/// Auth: GitLab PATs use the `PRIVATE-TOKEN` header — configure it via
/// `upstream_auth` as a custom header. OAuth `Authorization: Bearer` also works.
pub struct GitlabRegistryClient {
    pub(super) http: reqwest::Client,
    /// Instance root, e.g. `https://gitlab.com` (no trailing slash). Used for
    /// package-registry passthrough.
    pub(super) root: String,
    /// API base, derived as `{instance_root}/api/v4`.
    pub(super) api_base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
}

impl GitlabRegistryClient {
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

        let root = base_url.into();
        let root = root.trim_end_matches('/');
        let root = root
            .trim_end_matches("/api/v4")
            .trim_end_matches('/')
            .to_owned();
        let api_base_url = format!("{root}/api/v4");

        Ok(Self {
            http,
            root,
            api_base_url,
            basic_auth: opts.basic_auth.clone(),
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    /// Build the API project selector: the full project path, URL-encoded.
    pub(super) fn project_selector(project: &str) -> String {
        percent_encode(project)
    }

    /// Fetch every release for `project`, following `Link: rel="next"` pagination
    /// (GitLab default 20 per page; we request 100 and cap at 20 pages). Returns
    /// `NotFound` if the project's releases endpoint 404s on the first page.
    pub(super) async fn fetch_all_releases(
        &self,
        project: &str,
    ) -> Result<Vec<GlRelease>, CoreError> {
        use super::super::http_client::next_link;
        let mut url = format!(
            "{}/projects/{}/releases?per_page=100",
            self.api_base_url,
            Self::project_selector(project),
        );
        let mut all = Vec::new();
        for page in 0..20 {
            let resp = self.get(&url).send().await.map_err(to_registry_error)?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                if page == 0 {
                    return Err(CoreError::NotFound(format!("{project} not found")));
                }
                break;
            }
            if !resp.status().is_success() {
                return Err(CoreError::Registry(format!(
                    "gitlab: releases list returned {}",
                    resp.status()
                )));
            }
            let next = next_link(resp.headers());
            let releases: Vec<GlRelease> = resp.json().await.map_err(to_registry_error)?;
            all.extend(releases);
            match next {
                Some(n) => url = n,
                None => break,
            }
        }
        Ok(all)
    }

    /// Raw-file download URL via the repository files API:
    /// `{api}/projects/{enc}/repository/files/{enc(path)}/raw?ref={ref}`.
    pub(super) fn raw_file_url(&self, project: &str, git_ref: &str, path: &str) -> String {
        format!(
            "{}/projects/{}/repository/files/{}/raw?ref={}",
            self.api_base_url,
            Self::project_selector(project),
            percent_encode(path),
            percent_encode(git_ref),
        )
    }

    /// Package-registry passthrough URL: `{instance_root}/{relative}` (the relative
    /// path already includes `api/v4/...`).
    pub(super) fn passthrough_url(&self, relative: &str) -> String {
        format!("{}/{}", self.root, relative)
    }

    pub(super) async fn fetch_release_by_tag(
        &self,
        project: &str,
        tag: &str,
    ) -> Result<GlRelease, CoreError> {
        let url = format!(
            "{}/projects/{}/releases/{}",
            self.api_base_url,
            Self::project_selector(project),
            percent_encode(tag),
        );
        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{project}@{tag} not found")));
        }

        resp.error_for_status()
            .map_err(to_registry_error)?
            .json::<GlRelease>()
            .await
            .map_err(to_registry_error)
    }

    /// Source-archive download URL via the repository archive endpoint.
    pub(super) fn source_archive_url(&self, project: &str, tag: &str, format: &str) -> String {
        format!(
            "{}/projects/{}/repository/archive.{}?sha={}",
            self.api_base_url,
            Self::project_selector(project),
            format,
            percent_encode(tag),
        )
    }
}

// ── Pure helpers ──────────────────────────────────────────────────────────────

pub(super) fn is_release_signed(links: &[GlLink]) -> bool {
    links
        .iter()
        .any(|l| l.name.ends_with(".asc") || l.name.ends_with(".sig"))
}

/// If `artifact` selects a source archive (`source/{format}`), return the format.
pub(super) fn source_format(artifact: &str) -> Option<&str> {
    artifact.strip_prefix("source/")
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

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::ports::RegistryClient;

    #[test]
    fn project_selector_encodes_slashes() {
        assert_eq!(
            GitlabRegistryClient::project_selector("group/sub/proj"),
            "group%2Fsub%2Fproj"
        );
    }

    #[test]
    fn new_derives_api_base() {
        let opts = UpstreamHttpOptions::default();
        let client = GitlabRegistryClient::new("https://gitlab.com/", &opts).unwrap();
        assert_eq!(client.api_base_url, "https://gitlab.com/api/v4");
    }

    #[test]
    fn source_format_extracts() {
        assert_eq!(source_format("source/tar.gz"), Some("tar.gz"));
        assert_eq!(source_format("link/foo"), None);
    }

    #[test]
    fn raw_file_and_passthrough_urls() {
        let opts = UpstreamHttpOptions::default();
        let client = GitlabRegistryClient::new("https://gitlab.com", &opts).unwrap();
        assert_eq!(
            client.raw_file_url("grp/proj", "main", "src/x.rs"),
            "https://gitlab.com/api/v4/projects/grp%2Fproj/repository/files/src%2Fx.rs/raw?ref=main"
        );
        assert_eq!(
            client.passthrough_url("api/v4/projects/1/packages/generic/a/1.0/f.bin"),
            "https://gitlab.com/api/v4/projects/1/packages/generic/a/1.0/f.bin"
        );
    }

    #[tokio::test]
    async fn list_versions_follows_pagination() {
        let mut server = mockito::Server::new_async().await;
        let p2 = format!(
            "{}/api/v4/projects/grp%2Fproj/releases?per_page=100&page=2",
            server.url()
        );
        let _m1 = server
            .mock("GET", "/api/v4/projects/grp%2Fproj/releases?per_page=100")
            .with_status(200)
            .with_header("link", &format!(r#"<{p2}>; rel="next""#))
            .with_body(r#"[{"tag_name":"v2.0.0","assets":{"links":[],"sources":[]}}]"#)
            .create_async()
            .await;
        let _m2 = server
            .mock(
                "GET",
                "/api/v4/projects/grp%2Fproj/releases?per_page=100&page=2",
            )
            .with_status(200)
            .with_body(r#"[{"tag_name":"v1.0.0","assets":{"links":[],"sources":[]}}]"#)
            .create_async()
            .await;
        let client =
            GitlabRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let versions = client.list_versions("grp/proj").await.unwrap();
        assert_eq!(versions, vec!["v2.0.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn fetch_artifact_rawfile_and_package_passthrough() {
        let mut server = mockito::Server::new_async().await;
        let _raw = server
            .mock(
                "GET",
                "/api/v4/projects/grp%2Fproj/repository/files/README.md/raw?ref=main",
            )
            .with_status(200)
            .with_body(b"# readme")
            .create_async()
            .await;
        let _pkg = server
            .mock("GET", "/api/v4/projects/1/packages/generic/a/1.0/f.bin")
            .with_status(200)
            .with_body(b"PKG")
            .create_async()
            .await;
        let client =
            GitlabRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();

        let raw = batlehub_core::entities::PackageId::new("gl", "grp/proj", "main")
            .with_artifact("rawfile/main/README.md");
        let body = collect(client.fetch_artifact(&raw).await.unwrap()).await;
        assert_eq!(body, b"# readme");

        let pkg = batlehub_core::entities::PackageId::new("gl", "_packages", "_")
            .with_artifact("pkgpath/api/v4/projects/1/packages/generic/a/1.0/f.bin");
        let body = collect(client.fetch_artifact(&pkg).await.unwrap()).await;
        assert_eq!(body, b"PKG");
    }

    async fn collect(f: batlehub_core::ports::FetchedArtifact) -> Vec<u8> {
        futures::TryStreamExt::try_fold(f.stream, Vec::new(), |mut a, c| async move {
            a.extend_from_slice(&c);
            Ok(a)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn list_versions_returns_tags() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!([
            { "tag_name": "v1.1.0", "released_at": "2024-01-02T00:00:00Z", "assets": { "links": [], "sources": [] } },
            { "tag_name": "v1.0.0", "released_at": "2024-01-01T00:00:00Z", "assets": { "links": [], "sources": [] } },
        ]))
        .unwrap();
        let _mock = server
            .mock("GET", "/api/v4/projects/grp%2Fproj/releases?per_page=100")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = GitlabRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("grp/proj").await.unwrap();
        assert_eq!(versions, vec!["v1.1.0", "v1.0.0"]);
    }

    #[tokio::test]
    async fn fetch_artifact_source_archive_streams() {
        let mut server = mockito::Server::new_async().await;
        let _m = server
            .mock(
                "GET",
                "/api/v4/projects/grp%2Fproj/repository/archive.tar.gz?sha=v1.0.0",
            )
            .with_status(200)
            .with_body(b"SRC")
            .create_async()
            .await;
        let client =
            GitlabRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("gl", "grp/proj", "v1.0.0")
            .with_artifact("source/tar.gz");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let body =
            futures::TryStreamExt::try_fold(fetched.stream, Vec::new(), |mut a, c| async move {
                a.extend_from_slice(&c);
                Ok(a)
            })
            .await
            .unwrap();
        assert_eq!(body, b"SRC");
    }

    #[tokio::test]
    async fn fetch_artifact_link_resolves_via_release() {
        let mut server = mockito::Server::new_async().await;
        let dl = format!("{}/d/app.bin", server.url());
        let rel = serde_json::to_string(&serde_json::json!({
            "tag_name": "v1.0.0", "released_at": null,
            "assets": { "links": [ { "name": "app.bin", "url": dl } ], "sources": [] }
        }))
        .unwrap();
        let _m1 = server
            .mock("GET", "/api/v4/projects/grp%2Fproj/releases/v1.0.0")
            .with_status(200)
            .with_body(&rel)
            .create_async()
            .await;
        let _m2 = server
            .mock("GET", "/d/app.bin")
            .with_status(200)
            .with_body(b"BIN")
            .create_async()
            .await;
        let client =
            GitlabRegistryClient::new(server.url(), &UpstreamHttpOptions::default()).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("gl", "grp/proj", "v1.0.0")
            .with_artifact("link/app.bin");
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
    async fn resolve_metadata_by_tag_detects_signature_link() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!({
            "tag_name": "v2.0.0",
            "released_at": "2024-05-01T00:00:00Z",
            "assets": {
                "links": [
                    { "name": "app.bin", "url": "https://dl/app.bin", "direct_asset_url": "https://dl/d/app.bin" },
                    { "name": "app.bin.asc", "url": "https://dl/app.bin.asc" },
                ],
                "sources": [ { "format": "tar.gz", "url": "https://dl/src.tar.gz" } ]
            }
        }))
        .unwrap();
        let _mock = server
            .mock("GET", "/api/v4/projects/grp%2Fproj/releases/v2.0.0")
            .with_status(200)
            .with_body(&body)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = GitlabRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = batlehub_core::entities::PackageId::new("gl", "grp/proj", "v2.0.0");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.is_signed, Some(true));
        assert!(meta.published_at.is_some());
    }
}
