use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// VS Code Marketplace registry client (marketplace.visualstudio.com or compatible).
///
/// Supported `PackageId` conventions:
/// - `name` format: `"{publisher}.{extension}"` (e.g. `"ms-python.python"`)
/// - `version = "latest"` → current latest version
/// - `version = "1.2.3"`  → specific semver version
/// - `artifact = Some("vsix")` → stream the `.vsix` extension package
pub struct VsCodeMarketplaceRegistryClient {
    http: reqwest::Client,
    base_url: String,
}

impl VsCodeMarketplaceRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self { http, base_url: base_url.into() })
    }

    fn parse_id(name: &str) -> Result<(&str, &str), CoreError> {
        name.split_once('.').ok_or_else(|| {
            CoreError::Registry(format!(
                "invalid VS Code Marketplace extension id '{name}': expected '{{publisher}}.{{name}}'"
            ))
        })
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ExtensionQueryRequest {
    filters: Vec<ExtensionQueryFilter>,
    flags: u32,
}

#[derive(Serialize)]
struct ExtensionQueryFilter {
    criteria: Vec<ExtensionQueryCriteria>,
}

#[derive(Serialize)]
struct ExtensionQueryCriteria {
    #[serde(rename = "filterType")]
    filter_type: u32,
    value: String,
}

#[derive(Debug, Deserialize)]
struct ExtensionQueryResponse {
    results: Vec<ExtensionQueryResult>,
}

#[derive(Debug, Deserialize)]
struct ExtensionQueryResult {
    extensions: Vec<VSCodeExtension>,
}

#[derive(Debug, Deserialize)]
struct VSCodeExtension {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "shortDescription")]
    description: Option<String>,
    versions: Vec<VSCodeExtensionVersion>,
}

#[derive(Debug, Deserialize)]
struct VSCodeExtensionVersion {
    version: String,
    #[serde(rename = "lastUpdated")]
    last_updated: Option<String>,
    #[serde(default)]
    files: Vec<VSCodeExtensionFile>,
}

#[derive(Debug, Deserialize)]
struct VSCodeExtensionFile {
    #[serde(rename = "assetType")]
    asset_type: String,
    source: String,
}

struct ResolvedExtension {
    version_info: VSCodeExtensionVersion,
    display_name: Option<String>,
    description: Option<String>,
}

const VSIX_ASSET_TYPE: &str = "Microsoft.VisualStudio.Services.VSIXPackage";
const GALLERY_API_ACCEPT: &str = "application/json;api-version=3.0-preview.1";

// filterType values for extensionquery criteria
const FILTER_EXTENSION_NAME: u32 = 7;
const FILTER_VERSION: u32 = 10;

// flags bitmask for extensionquery
const FLAG_INCLUDE_VERSIONS: u32 = 0x001;
const FLAG_INCLUDE_FILES: u32 = 0x002;
const FLAG_INCLUDE_ASSET_URI: u32 = 0x080;
const FLAG_INCLUDE_LATEST_ONLY: u32 = 0x200;

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for VsCodeMarketplaceRegistryClient {
    fn registry_type(&self) -> &str {
        "vscode-marketplace"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;
        let resolved = self.query_extension(publisher, ext_name, &pkg.version).await?;

        let published_at = resolved
            .version_info
            .last_updated
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let vsix_url = resolved
            .version_info
            .files
            .iter()
            .find(|f| f.asset_type == VSIX_ASSET_TYPE)
            .map(|f| f.source.clone());

        let download_url = if pkg.artifact.as_deref() == Some("vsix") { vsix_url } else { None };

        let extra = serde_json::json!({
            "resolved_version": resolved.version_info.version,
            "display_name": resolved.display_name,
            "description": resolved.description,
        });

        Ok(PackageMetadata {
            id: PackageId { version: resolved.version_info.version, ..pkg.clone() },
            published_at,
            download_url,
            checksum: None,
            is_signed: Some(false),
            extra,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;

        let url = format!(
            "{base}/_apis/public/gallery/publishers/{publisher}/vsextensions/{name}/{version}/vspackage",
            base = self.base_url,
            name = ext_name,
            version = pkg.version,
        );

        tracing::debug!(url = %url, "fetching VS Code Marketplace VSIX");

        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "VS Code Marketplace extension {publisher}.{ext_name}@{} not found",
                pkg.version,
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

impl VsCodeMarketplaceRegistryClient {
    async fn query_extension(
        &self,
        publisher: &str,
        name: &str,
        version: &str,
    ) -> Result<ResolvedExtension, CoreError> {
        let (flags, criteria) = if version == "latest" {
            (
                FLAG_INCLUDE_VERSIONS | FLAG_INCLUDE_FILES | FLAG_INCLUDE_ASSET_URI
                    | FLAG_INCLUDE_LATEST_ONLY,
                vec![ExtensionQueryCriteria {
                    filter_type: FILTER_EXTENSION_NAME,
                    value: format!("{publisher}.{name}"),
                }],
            )
        } else {
            (
                FLAG_INCLUDE_VERSIONS | FLAG_INCLUDE_FILES | FLAG_INCLUDE_ASSET_URI,
                vec![
                    ExtensionQueryCriteria {
                        filter_type: FILTER_EXTENSION_NAME,
                        value: format!("{publisher}.{name}"),
                    },
                    ExtensionQueryCriteria {
                        filter_type: FILTER_VERSION,
                        value: version.to_owned(),
                    },
                ],
            )
        };

        let body = ExtensionQueryRequest {
            filters: vec![ExtensionQueryFilter { criteria }],
            flags,
        };

        let url = format!("{}/_apis/public/gallery/extensionquery", self.base_url);

        let resp = self
            .http
            .post(&url)
            .header("Accept", GALLERY_API_ACCEPT)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "VS Code Marketplace extension {publisher}.{name}@{version} not found"
            )));
        }

        let query_resp = resp
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<ExtensionQueryResponse>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        // The API returns 200 with empty results for missing extensions
        let ext = query_resp
            .results
            .into_iter()
            .next()
            .and_then(|r| r.extensions.into_iter().next())
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "VS Code Marketplace extension {publisher}.{name}@{version} not found"
                ))
            })?;

        let version_info = ext.versions.into_iter().next().ok_or_else(|| {
            CoreError::NotFound(format!(
                "no versions available for {publisher}.{name}@{version}"
            ))
        })?;

        Ok(ResolvedExtension {
            version_info,
            display_name: ext.display_name,
            description: ext.description,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn pkg(name: &str, version: &str) -> PackageId {
        PackageId::new("vscode-marketplace", name, version)
    }

    fn ext_body(version: &str) -> String {
        format!(
            r#"{{"results":[{{"extensions":[{{"displayName":"Python","shortDescription":"Python language support","publisher":{{"publisherName":"ms-python"}},"versions":[{{"version":"{version}","lastUpdated":"2024-01-01T00:00:00Z","files":[{{"assetType":"Microsoft.VisualStudio.Services.VSIXPackage","source":"http://example.com/python.vsix"}}]}}]}}]}}]}}"#
        )
    }

    const EMPTY_RESULTS: &str = r#"{"results":[{"extensions":[]}]}"#;

    // ── parse_id ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_id_valid() {
        let (publisher, name) =
            VsCodeMarketplaceRegistryClient::parse_id("ms-python.python").unwrap();
        assert_eq!(publisher, "ms-python");
        assert_eq!(name, "python");
    }

    #[test]
    fn parse_id_multiple_dots() {
        let (publisher, name) =
            VsCodeMarketplaceRegistryClient::parse_id("pub.ext.extra").unwrap();
        assert_eq!(publisher, "pub");
        assert_eq!(name, "ext.extra");
    }

    #[test]
    fn parse_id_no_dot() {
        let result = VsCodeMarketplaceRegistryClient::parse_id("nopublisher");
        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    // ── resolve_metadata ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_metadata_latest() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("ms-python.python", "latest")).await.unwrap();

        assert_eq!(meta.id.version, "2024.2.1");
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta =
            client.resolve_metadata(&pkg("ms-python.python", "2024.2.1")).await.unwrap();

        assert_eq!(meta.id.version, "2024.2.1");
    }

    #[tokio::test]
    async fn resolve_metadata_timestamp_parsed() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("ms-python.python", "latest")).await.unwrap();

        assert!(meta.published_at.is_some());
    }

    #[tokio::test]
    async fn resolve_metadata_no_artifact() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("ms-python.python", "2024.2.1")).await.unwrap();

        assert!(meta.download_url.is_none());
    }

    #[tokio::test]
    async fn resolve_metadata_vsix_artifact() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let p = pkg("ms-python.python", "2024.2.1").with_artifact("vsix");
        let meta = client.resolve_metadata(&p).await.unwrap();

        assert_eq!(meta.download_url.as_deref(), Some("http://example.com/python.vsix"));
    }

    #[tokio::test]
    async fn resolve_metadata_is_signed_false() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(ext_body("2024.2.1"))
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let meta = client.resolve_metadata(&pkg("ms-python.python", "2024.2.1")).await.unwrap();

        assert_eq!(meta.is_signed, Some(false));
    }

    #[tokio::test]
    async fn resolve_metadata_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(EMPTY_RESULTS)
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.resolve_metadata(&pkg("ms-python.python", "9.9.9")).await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn resolve_metadata_server_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("POST", "/_apis/public/gallery/extensionquery")
            .with_status(500)
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.resolve_metadata(&pkg("ms-python.python", "2024.2.1")).await;

        assert!(matches!(result, Err(CoreError::Registry(_))));
    }

    // ── fetch_artifact ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_artifact_streams_bytes() {
        let mut server = Server::new_async().await;
        let dl_path =
            "/_apis/public/gallery/publishers/ms-python/vsextensions/python/2024.2.1/vspackage";
        let _mock = server
            .mock("GET", dl_path)
            .with_status(200)
            .with_body("fake vsix content")
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let stream =
            client.fetch_artifact(&pkg("ms-python.python", "2024.2.1")).await.unwrap();
        let chunks: Vec<bytes::Bytes> = stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, b"fake vsix content");
    }

    #[tokio::test]
    async fn fetch_artifact_not_found() {
        let mut server = Server::new_async().await;
        let dl_path =
            "/_apis/public/gallery/publishers/ms-python/vsextensions/python/9.9.9/vspackage";
        let _mock = server
            .mock("GET", dl_path)
            .with_status(404)
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.fetch_artifact(&pkg("ms-python.python", "9.9.9")).await;

        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_server_error() {
        let mut server = Server::new_async().await;
        let dl_path =
            "/_apis/public/gallery/publishers/ms-python/vsextensions/python/2024.2.1/vspackage";
        let _mock = server
            .mock("GET", dl_path)
            .with_status(500)
            .create_async()
            .await;

        let client = VsCodeMarketplaceRegistryClient::new(server.url(), &Default::default()).unwrap();
        let result = client.fetch_artifact(&pkg("ms-python.python", "2024.2.1")).await;

        assert!(matches!(result, Err(CoreError::Registry(_))));
    }
}
