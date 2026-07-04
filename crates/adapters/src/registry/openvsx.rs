use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{
    basic_auth_get, cache_control, ensure_same_origin, new_http_client, percent_encode,
    to_registry_error, UpstreamHttpOptions,
};

/// OpenVSX registry client (open-vsx.org or compatible).
///
/// Supported `PackageId` conventions:
/// - `name` format: `"{publisher}.{extension}"` (e.g. `"ms-python.python"`)
/// - `version = "latest"` → current latest version
/// - `version = "1.2.3"`  → specific semver version
/// - `artifact = Some("vsix")` → stream the `.vsix` extension package
pub struct OpenVsxRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl OpenVsxRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let http = new_http_client(Some(10), opts)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
        })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    fn parse_id(name: &str) -> Result<(&str, &str), CoreError> {
        name.split_once('.').ok_or_else(|| {
            CoreError::Registry(format!(
                "invalid OpenVSX extension id '{name}': expected '{{publisher}}.{{name}}'"
            ))
        })
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenVsxExtension {
    namespace: String,
    #[allow(dead_code)]
    name: String,
    version: String,
    timestamp: Option<String>,
    #[serde(default)]
    files: OpenVsxFiles,
    #[serde(rename = "allVersions", default)]
    all_versions: HashMap<String, String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    description: Option<String>,
    #[serde(rename = "downloadCount", default)]
    download_count: u64,
    verified: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenVsxFiles {
    download: Option<String>,
    signature: Option<String>,
    manifest: Option<String>,
    icon: Option<String>,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for OpenVsxRegistryClient {
    fn registry_type(&self) -> &str {
        "openvsx"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;
        let ext = self
            .fetch_extension(publisher, ext_name, &pkg.version)
            .await?;

        let published_at = ext
            .timestamp
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let download_url = if pkg.artifact.as_deref() == Some("vsix") {
            ext.files.download.clone()
        } else {
            None
        };

        let is_signed = Some(ext.files.signature.is_some());

        let extra = serde_json::json!({
            "resolved_version": ext.version,
            "namespace": ext.namespace,
            "display_name": ext.display_name,
            "description": ext.description,
            "download_count": ext.download_count,
            "verified": ext.verified,
            "manifest_url": ext.files.manifest,
            "icon_url": ext.files.icon,
            "all_versions_count": ext.all_versions.len(),
        });

        Ok(PackageMetadata {
            id: PackageId {
                version: ext.version,
                ..pkg.clone()
            },
            published_at,
            download_url,
            checksum: None,
            is_signed,
            extra,
            cache_control: None,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let (publisher, ext_name) = Self::parse_id(package)?;
        let ext = self.fetch_extension(publisher, ext_name, "latest").await?;
        let mut versions: Vec<String> = ext.all_versions.into_keys().collect();
        versions.sort();
        Ok(versions)
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;
        let ext = self
            .fetch_extension(publisher, ext_name, &pkg.version)
            .await?;

        let download_url = ext.files.download.ok_or_else(|| {
            CoreError::NotFound(format!(
                "no VSIX download available for {}.{} v{}",
                publisher, ext_name, pkg.version
            ))
        })?;

        ensure_same_origin(&download_url, &self.base_url)?;
        tracing::debug!(url = %download_url, "fetching OpenVSX VSIX");

        let response = self
            .get(&download_url)
            .send()
            .await
            .map_err(to_registry_error)?
            .error_for_status()
            .map_err(to_registry_error)?;

        let cache_control = cache_control(&response);

        let stream = response.bytes_stream().map_err(to_registry_error);

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(Deserialize)]
        struct SearchResponse {
            extensions: Vec<ExtHit>,
        }
        #[derive(Deserialize)]
        struct ExtHit {
            namespace: String,
            name: String,
            version: String,
            description: Option<String>,
        }

        // Strip any /api suffix that some configs include in the base URL,
        // so we don't produce .../api/api/-/search.
        let base = self
            .base_url
            .trim_end_matches('/')
            .trim_end_matches("/api")
            .trim_end_matches('/');

        let url = format!(
            "{}/api/-/search?query={}&size={}",
            base,
            percent_encode(query),
            limit.min(50),
        );

        let res = self.get(&url).send().await.map_err(to_registry_error)?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = res.json().await.map_err(to_registry_error)?;

        Ok(body
            .extensions
            .into_iter()
            .map(|e| UpstreamPackage {
                // OpenVSX package IDs use "publisher.name" (dot separator)
                name: format!("{}.{}", e.namespace, e.name),
                latest_version: e.version,
                description: e.description,
            })
            .collect())
    }
}

impl OpenVsxRegistryClient {
    async fn fetch_extension(
        &self,
        publisher: &str,
        name: &str,
        version: &str,
    ) -> Result<OpenVsxExtension, CoreError> {
        let url = if version == "latest" {
            format!("{}/api/{}/{}", self.base_url, publisher, name)
        } else {
            format!("{}/api/{}/{}/{}", self.base_url, publisher, name, version)
        };

        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "OpenVSX extension {publisher}.{name}@{version} not found"
            )));
        }

        resp.error_for_status()
            .map_err(to_registry_error)?
            .json::<OpenVsxExtension>()
            .await
            .map_err(to_registry_error)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn pkg(name: &str, version: &str) -> PackageId {
        PackageId::new("openvsx", name, version)
    }

    const EXT_BODY: &str = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0"}"#;

    // ── parse_id ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_id_valid() {
        let (publisher, name) = OpenVsxRegistryClient::parse_id("ms-python.python").unwrap();
        assert_eq!(publisher, "ms-python");
        assert_eq!(name, "python");
    }

    #[test]
    fn parse_id_multiple_dots() {
        // split_once stops at the first dot, so extra dots go into the name segment
        let (publisher, name) = OpenVsxRegistryClient::parse_id("pub.ext.extra").unwrap();
        assert_eq!(publisher, "pub");
        assert_eq!(name, "ext.extra");
    }

    #[test]
    fn parse_id_no_dot() {
        let result = OpenVsxRegistryClient::parse_id("nopublisher");
        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    // ── resolve_metadata ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_metadata_latest() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(EXT_BODY)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "latest"))
            .await
            .unwrap();

        assert_eq!(meta.id.version, "2023.20.0");
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(EXT_BODY)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();

        assert_eq!(meta.id.version, "2023.20.0");
    }

    #[tokio::test]
    async fn resolve_metadata_no_artifact() {
        let mut server = Server::new_async().await;
        let body = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{"download":"http://example.com/ext.vsix"}}"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();

        assert!(meta.download_url.is_none());
    }

    #[tokio::test]
    async fn resolve_metadata_vsix_artifact() {
        let mut server = Server::new_async().await;
        let body = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{"download":"http://example.com/ext.vsix"}}"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let p = pkg("ms-python.python", "2023.20.0").with_artifact("vsix");
        let meta = client.resolve_metadata(&p).await.unwrap();

        assert_eq!(
            meta.download_url.as_deref(),
            Some("http://example.com/ext.vsix")
        );
    }

    #[tokio::test]
    async fn resolve_metadata_is_signed_true() {
        let mut server = Server::new_async().await;
        let body = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{"signature":"http://example.com/ext.sigzip"}}"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();

        assert_eq!(meta.is_signed, Some(true));
    }

    #[tokio::test]
    async fn resolve_metadata_is_signed_false() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(EXT_BODY)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();

        assert_eq!(meta.is_signed, Some(false));
    }

    #[tokio::test]
    async fn resolve_metadata_timestamp_parsed() {
        let mut server = Server::new_async().await;
        let body = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0","timestamp":"2023-11-09T18:25:45Z"}"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client
            .resolve_metadata(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();

        assert!(meta.published_at.is_some());
    }

    #[tokio::test]
    async fn resolve_metadata_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/9.9.9")
            .with_status(404)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .resolve_metadata(&pkg("ms-python.python", "9.9.9"))
            .await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    // ── fetch_artifact ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_artifact_streams_bytes() {
        let mut server = Server::new_async().await;
        let dl_path = "/files/ms-python/python/2023.20.0/python.vsix";
        let dl_url = format!("{}{}", server.url(), dl_path);
        let body = format!(
            r#"{{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{{"download":"{}"}}}}"#,
            dl_url
        );

        let _mock_meta = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let _mock_dl = server
            .mock("GET", dl_path)
            .with_status(200)
            .with_body("fake vsix content")
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let fetched = client
            .fetch_artifact(&pkg("ms-python.python", "2023.20.0"))
            .await
            .unwrap();
        let chunks: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, b"fake vsix content");
    }

    #[tokio::test]
    async fn fetch_artifact_missing_download_url() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(EXT_BODY)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .fetch_artifact(&pkg("ms-python.python", "2023.20.0"))
            .await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_extension_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/9.9.9")
            .with_status(404)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .fetch_artifact(&pkg("ms-python.python", "9.9.9"))
            .await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_extension_server_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(500)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .fetch_artifact(&pkg("ms-python.python", "2023.20.0"))
            .await;

        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    // ── list_versions ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_versions_returns_sorted_versions() {
        let mut server = Server::new_async().await;
        let body = r#"{
            "namespace":"ms-python","name":"python","version":"2024.1.0",
            "allVersions":{
                "2024.1.0":"http://example.com/2024.1.0",
                "2023.20.0":"http://example.com/2023.20.0",
                "2023.5.0":"http://example.com/2023.5.0"
            }
        }"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let versions = client.list_versions("ms-python.python").await.unwrap();

        assert_eq!(versions, vec!["2023.20.0", "2023.5.0", "2024.1.0"]);
    }

    #[tokio::test]
    async fn list_versions_empty_when_extension_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/ms-python/unknown")
            .with_status(404)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.list_versions("ms-python.unknown").await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_versions_invalid_id_returns_error() {
        let client = OpenVsxRegistryClient::new("http://unused", &Default::default()).unwrap();
        let result = client.list_versions("nodot").await;
        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_download_server_error() {
        let mut server = Server::new_async().await;
        let dl_path = "/files/ms-python/python/2023.20.0/python.vsix";
        let dl_url = format!("{}{}", server.url(), dl_path);
        let body = format!(
            r#"{{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{{"download":"{}"}}}}"#,
            dl_url
        );

        let _mock_meta = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;
        let _mock_dl = server
            .mock("GET", dl_path)
            .with_status(500)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .fetch_artifact(&pkg("ms-python.python", "2023.20.0"))
            .await;

        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_rejects_cross_origin_download_url() {
        let mut server = Server::new_async().await;
        let body = r#"{"namespace":"ms-python","name":"python","version":"2023.20.0","files":{"download":"http://evil.example.com/ext.vsix"}}"#;
        let _mock = server
            .mock("GET", "/api/ms-python/python/2023.20.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = OpenVsxRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client
            .fetch_artifact(&pkg("ms-python.python", "2023.20.0"))
            .await;

        assert!(matches!(result, Err(CoreError::Registry(_))));
    }
}
