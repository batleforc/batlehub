use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// Terraform provider and module registry proxy client.
///
/// Implements the Terraform Registry Protocol v1:
/// - Providers: <https://developer.hashicorp.com/terraform/internals/provider-registry-protocol>
/// - Modules:   <https://developer.hashicorp.com/terraform/internals/module-registry-protocol>
///
/// Default upstream: `https://registry.terraform.io`
///
/// Supported `PackageId` conventions:
/// - `name` encodes the entity type and path:
///   - `"providers/{namespace}/{type}"` (e.g. `"providers/hashicorp/aws"`)
///   - `"modules/{namespace}/{name}/{provider}"` (e.g. `"modules/hashicorp/consul/aws"`)
/// - `version`:
///   - `"versions"` → fetch the full versions listing JSON
///   - `"{semver}"` → target a specific version
/// - `artifact`:
///   - `None`                → versions listing (when `version = "versions"`)
///   - `Some("{os}/{arch}")` → provider download info JSON for that platform
///   - `Some("download")`   → module source download (returns redirect URL via `X-Terraform-Get`)
pub struct TerraformRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl TerraformRegistryClient {
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

    /// Build the upstream URL for the given `PackageId`.
    fn artifact_url(&self, pkg: &PackageId) -> Result<String, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let is_provider = pkg.name.starts_with("providers/");
        let is_module = pkg.name.starts_with("modules/");

        if !is_provider && !is_module {
            return Err(CoreError::Registry(format!(
                "terraform: invalid package name '{}': must start with 'providers/' or 'modules/'",
                pkg.name
            )));
        }

        if pkg.version == "versions" {
            return Ok(format!("{base}/v1/{}/versions", pkg.name));
        }

        if is_provider {
            match pkg.artifact.as_deref() {
                None => {
                    // Provider version metadata (not a standard endpoint; fall back to versions)
                    Ok(format!("{base}/v1/{}/versions", pkg.name))
                }
                Some(platform) => {
                    // platform = "linux/amd64"
                    let (os, arch) = platform.split_once('/').ok_or_else(|| {
                        CoreError::Registry(format!(
                            "terraform: invalid provider platform '{platform}': expected 'os/arch'"
                        ))
                    })?;
                    Ok(format!("{base}/v1/{}/{}/download/{os}/{arch}", pkg.name, pkg.version))
                }
            }
        } else {
            // module
            match pkg.artifact.as_deref() {
                Some("download") | None => {
                    Ok(format!("{base}/v1/{}/{}/download", pkg.name, pkg.version))
                }
                Some(other) => Err(CoreError::Registry(format!(
                    "terraform: unknown module artifact '{other}'"
                ))),
            }
        }
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TfProviderVersions {
    versions: Vec<TfProviderVersion>,
}

#[derive(Debug, Deserialize)]
struct TfProviderVersion {
    version: String,
}

#[derive(Debug, Deserialize)]
struct TfModuleVersions {
    modules: Vec<TfModuleEntry>,
}

#[derive(Debug, Deserialize)]
struct TfModuleEntry {
    versions: Vec<TfModuleVersion>,
}

#[derive(Debug, Deserialize)]
struct TfModuleVersion {
    version: String,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for TerraformRegistryClient {
    fn registry_type(&self) -> &str {
        "terraform"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        // For metadata resolution we perform a lightweight HEAD request to confirm
        // the resource exists and capture Cache-Control from the upstream.
        let url = self.artifact_url(pkg)?;

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("terraform metadata request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "terraform resource not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(CoreError::Registry(format!(
                "terraform metadata request returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: None,
            download_url: Some(url),
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let url = self.artifact_url(pkg)?;

        tracing::debug!(url = %url, "fetching Terraform artifact");

        let response = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "terraform artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !response.status().is_success() && response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(CoreError::Registry(format!(
                "terraform upstream returned {} for {}",
                response.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = response
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = response
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact { stream: Box::pin(stream), cache_control })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/v1/{package}/versions");

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }

        let body = resp
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if package.starts_with("providers/") {
            let parsed: TfProviderVersions = serde_json::from_slice(&body)
                .map_err(|e| CoreError::Registry(format!("parsing provider versions: {e}")))?;
            Ok(parsed.versions.into_iter().map(|v| v.version).collect())
        } else if package.starts_with("modules/") {
            let parsed: TfModuleVersions = serde_json::from_slice(&body)
                .map_err(|e| CoreError::Registry(format!("parsing module versions: {e}")))?;
            Ok(parsed
                .modules
                .into_iter()
                .flat_map(|m| m.versions.into_iter().map(|v| v.version))
                .collect())
        } else {
            Err(CoreError::Registry(format!(
                "terraform: package '{package}' must start with 'providers/' or 'modules/'"
            )))
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn provider_pkg(namespace: &str, ptype: &str, version: &str) -> PackageId {
        PackageId::new("tf", format!("providers/{namespace}/{ptype}"), version)
    }

    fn module_pkg(namespace: &str, name: &str, provider: &str, version: &str) -> PackageId {
        PackageId::new("tf", format!("modules/{namespace}/{name}/{provider}"), version)
    }

    #[tokio::test]
    async fn list_versions_providers() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/hashicorp/aws/versions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"versions":[{"version":"5.0.0"},{"version":"4.67.0"}]}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let versions = client.list_versions("providers/hashicorp/aws").await.unwrap();
        assert_eq!(versions, vec!["5.0.0", "4.67.0"]);
    }

    #[tokio::test]
    async fn list_versions_modules() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"modules":[{"versions":[{"version":"0.1.0"},{"version":"0.2.0"}]}]}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let versions = client.list_versions("modules/hashicorp/consul/aws").await.unwrap();
        assert_eq!(versions, vec!["0.1.0", "0.2.0"]);
    }

    #[tokio::test]
    async fn list_versions_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/example/unknown/versions")
            .with_status(404)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let versions = client.list_versions("providers/example/unknown").await.unwrap();
        assert!(versions.is_empty());
    }

    #[tokio::test]
    async fn fetch_artifact_provider_versions() {
        let body = r#"{"versions":[{"version":"5.0.0","protocols":["5.0"],"platforms":[]}]}"#;
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/hashicorp/aws/versions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "versions");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
        let content = bytes.into_iter().flat_map(|b| b.to_vec()).collect::<Vec<u8>>();
        assert!(String::from_utf8(content).unwrap().contains("5.0.0"));
    }

    #[tokio::test]
    async fn fetch_artifact_provider_download_info() {
        let body = r#"{"os":"linux","arch":"amd64","download_url":"https://releases.hashicorp.com/..."}"#;
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "5.0.0").with_artifact("linux/amd64");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
        let content = bytes.into_iter().flat_map(|b| b.to_vec()).collect::<Vec<u8>>();
        assert!(String::from_utf8(content).unwrap().contains("linux"));
    }

    #[tokio::test]
    async fn fetch_artifact_module_versions() {
        let body = r#"{"modules":[{"versions":[{"version":"0.1.0"}]}]}"#;
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = module_pkg("hashicorp", "consul", "aws", "versions");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
        let content = bytes.into_iter().flat_map(|b| b.to_vec()).collect::<Vec<u8>>();
        assert!(String::from_utf8(content).unwrap().contains("0.1.0"));
    }

    #[tokio::test]
    async fn fetch_artifact_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/example/unknown/versions")
            .with_status(404)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("example", "unknown", "versions");
        let result = client.fetch_artifact(&pkg).await;
        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn resolve_metadata_ok() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/hashicorp/aws/versions")
            .with_status(200)
            .with_header("cache-control", "max-age=300")
            .with_body(r#"{"versions":[]}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "versions");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.cache_control.as_deref(), Some("max-age=300"));
    }

    #[tokio::test]
    async fn resolve_metadata_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/example/missing/versions")
            .with_status(404)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("example", "missing", "versions");
        let result = client.resolve_metadata(&pkg).await;
        assert!(matches!(result, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn invalid_package_name() {
        let client =
            TerraformRegistryClient::new("https://registry.terraform.io", &Default::default())
                .unwrap();
        let pkg = PackageId::new("tf", "bad-name", "versions");
        assert!(matches!(client.fetch_artifact(&pkg).await, Err(CoreError::Registry(_))));
    }

    #[tokio::test]
    async fn provider_artifact_url_platform() {
        let client =
            TerraformRegistryClient::new("https://registry.terraform.io", &Default::default())
                .unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "5.0.0").with_artifact("linux/amd64");
        let url = client.artifact_url(&pkg).unwrap();
        assert_eq!(
            url,
            "https://registry.terraform.io/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64"
        );
    }
}
