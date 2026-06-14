use super::super::http_client::{
    apply_upstream_tls, basic_auth_get, percent_encode, upstream_auth_headers, UpstreamHttpOptions,
};
use super::models::{GlLink, GlRelease};
use batlehub_core::error::CoreError;

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
        let root = root.trim_end_matches("/api/v4");
        let api_base_url = format!("{}/api/v4", root.trim_end_matches('/'));

        Ok(Self {
            http,
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
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("{project}@{tag} not found")));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<GlRelease>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
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

    #[tokio::test]
    async fn list_versions_returns_tags() {
        let mut server = mockito::Server::new_async().await;
        let body = serde_json::to_string(&serde_json::json!([
            { "tag_name": "v1.1.0", "released_at": "2024-01-02T00:00:00Z", "assets": { "links": [], "sources": [] } },
            { "tag_name": "v1.0.0", "released_at": "2024-01-01T00:00:00Z", "assets": { "links": [], "sources": [] } },
        ]))
        .unwrap();
        let _mock = server
            .mock("GET", "/api/v4/projects/grp%2Fproj/releases")
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
