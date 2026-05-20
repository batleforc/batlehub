use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// Go module proxy client (proxy.golang.org or compatible).
///
/// Implements the GOPROXY protocol: https://go.dev/ref/mod#goproxy-protocol
///
/// Supported `PackageId` conventions:
/// - `name` format: Go module path without trailing slash
///   (e.g. `"golang.org/x/text"`, `"github.com/!burnt!sushi/toml"`)
///   Uppercase-encoded paths (`!{lowercase}`) are passed through unchanged —
///   the upstream GOPROXY also expects the encoded form.
/// - `version = "latest"` → used for `@latest` and `@v/list` endpoints
/// - `version = "v1.2.3"` → specific version
/// - `artifact = None`         → stream `.info` JSON (or `@latest` JSON when version = "latest")
/// - `artifact = Some("list")` → stream `@v/list`
/// - `artifact = Some("mod")`  → stream `go.mod` file
/// - `artifact = Some("zip")`  → stream module source zip
///
/// **Caching note**: `@latest` and `@v/list` responses are cached permanently,
/// like all other artifacts. They may become stale after new module versions are
/// published. Clear the proxy storage to refresh, or pin versions in `go.sum`.
pub struct GoProxyRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl GoProxyRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self { http, base_url: base_url.into(), basic_auth: opts.basic_auth.clone() })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GoVersionInfo {
    #[serde(rename = "Version")]
    version: String,
    #[serde(rename = "Time")]
    time: Option<String>,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for GoProxyRegistryClient {
    fn registry_type(&self) -> &str {
        "goproxy"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let info = if pkg.version == "latest" {
            self.fetch_info_latest(&pkg.name).await?
        } else {
            self.fetch_info_version(&pkg.name, &pkg.version).await?
        };

        let published_at = info
            .time
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let extra = serde_json::json!({
            "resolved_version": info.version,
        });

        Ok(PackageMetadata {
            id: PackageId {
                version: info.version,
                ..pkg.clone()
            },
            published_at,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
        let url = match pkg.artifact.as_deref() {
            Some("list") => {
                format!("{}/@v/list", self.module_base(&pkg.name))
            }
            Some("mod") => {
                format!("{}/@v/{}.mod", self.module_base(&pkg.name), pkg.version)
            }
            Some("zip") => {
                format!("{}/@v/{}.zip", self.module_base(&pkg.name), pkg.version)
            }
            None if pkg.version == "latest" => {
                format!("{}/@latest", self.module_base(&pkg.name))
            }
            None => {
                format!("{}/@v/{}.info", self.module_base(&pkg.name), pkg.version)
            }
            Some(other) => {
                return Err(CoreError::Registry(format!(
                    "unknown goproxy artifact type '{other}'"
                )));
            }
        };

        tracing::debug!(url = %url, "fetching Go module artifact");

        let response = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "Go module {}/{} not found",
                pkg.name, pkg.version
            )));
        }

        let stream = response
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(Box::pin(stream))
    }
}

impl GoProxyRegistryClient {
    /// Returns `{base_url}/{module}` — the common prefix for all GOPROXY endpoints.
    fn module_base(&self, module: &str) -> String {
        format!("{}/{}", self.base_url, module)
    }

    async fn fetch_info_latest(&self, module: &str) -> Result<GoVersionInfo, CoreError> {
        let url = format!("{}/@latest", self.module_base(module));
        self.fetch_info_url(&url, module, "latest").await
    }

    async fn fetch_info_version(
        &self,
        module: &str,
        version: &str,
    ) -> Result<GoVersionInfo, CoreError> {
        let url = format!("{}/@v/{}.info", self.module_base(module), version);
        self.fetch_info_url(&url, module, version).await
    }

    async fn fetch_info_url(
        &self,
        url: &str,
        module: &str,
        version: &str,
    ) -> Result<GoVersionInfo, CoreError> {
        let resp = self
            .get(url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "Go module {module}@{version} not found"
            )));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<GoVersionInfo>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn pkg(name: &str, version: &str) -> PackageId {
        PackageId::new("go", name, version)
    }

    #[tokio::test]
    async fn resolve_metadata_latest() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/golang.org/x/text/@latest")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"Version":"v0.14.0","Time":"2023-11-09T18:25:45Z"}"#)
            .create_async()
            .await;

        let client = GoProxyRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("golang.org/x/text", "latest")).await.unwrap();

        assert_eq!(meta.id.version, "v0.14.0");
        assert!(meta.published_at.is_some());
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/golang.org/x/text/@v/v0.3.7.info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"Version":"v0.3.7","Time":"2021-06-17T00:00:00Z"}"#)
            .create_async()
            .await;

        let client = GoProxyRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("golang.org/x/text", "v0.3.7")).await.unwrap();

        assert_eq!(meta.id.version, "v0.3.7");
        assert!(meta.published_at.is_some());
    }

    #[tokio::test]
    async fn resolve_metadata_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/example.com/unknown/@latest")
            .with_status(404)
            .create_async()
            .await;

        let client = GoProxyRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.resolve_metadata(&pkg("example.com/unknown", "latest")).await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_info() {
        let mut server = Server::new_async().await;
        let body = r#"{"Version":"v0.3.7","Time":"2021-06-17T00:00:00Z"}"#;
        let _mock = server
            .mock("GET", "/golang.org/x/text/@v/v0.3.7.info")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let client = GoProxyRegistryClient::new(server.url(), &Default::default()).unwrap();
        let stream = client.fetch_artifact(&pkg("golang.org/x/text", "v0.3.7")).await.unwrap();
        let bytes: Vec<bytes::Bytes> = stream.try_collect().await.unwrap();
        let content = bytes.into_iter().flat_map(|b| b.to_vec()).collect::<Vec<u8>>();
        let content = String::from_utf8(content).unwrap();
        assert!(content.contains("v0.3.7"));
    }

    #[tokio::test]
    async fn fetch_artifact_list() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/golang.org/x/text/@v/list")
            .with_status(200)
            .with_body("v0.3.6\nv0.3.7\n")
            .create_async()
            .await;

        let client = GoProxyRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg_list = pkg("golang.org/x/text", "latest").with_artifact("list");
        let stream = client.fetch_artifact(&pkg_list).await.unwrap();
        let bytes: Vec<bytes::Bytes> = stream.try_collect().await.unwrap();
        let content = bytes.into_iter().flat_map(|b| b.to_vec()).collect::<Vec<u8>>();
        let content = String::from_utf8(content).unwrap();
        assert!(content.contains("v0.3.7"));
    }
}
