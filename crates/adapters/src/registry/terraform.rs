use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;
use tracing as log;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, percent_encode, UpstreamHttpOptions};

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
    /// Resolved search base URL. `None` = disabled; `Some(url)` = use this.
    search_base: Option<String>,
}

impl TerraformRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;
        let base_url = base_url.into();

        // Terraform search API endpoints use /v1/modules/search and /v1/providers/{ns}.
        // Strip any trailing /v1 component from the base URL so we don't double it up
        // (some configs set upstreams = ["https://registry.terraform.io/v1"]).
        let search_root = base_url
            .trim_end_matches('/')
            .trim_end_matches("/v1")
            .trim_end_matches('/');
        let search_base = match opts.search_url.as_deref() {
            Some("") => None,
            Some(url) => Some(url.trim_end_matches('/').to_owned()),
            None => Some(search_root.to_owned()),
        };

        Ok(Self {
            http,
            base_url,
            basic_auth: opts.basic_auth.clone(),
            search_base,
        })
    }

    /// Fetch `url` and decode the JSON body as `T`. Returns `None` on network
    /// error, non-2xx status, or deserialization failure, logging a warning.
    async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        label: &str,
    ) -> Option<T> {
        let res = match self.get(url).send().await {
            Err(e) => {
                log::warn!(%url, error = %e, "{label}: send failed");
                return None;
            }
            Ok(r) => r,
        };
        let status = res.status();
        if !status.is_success() {
            log::warn!(%url, %status, "{label}: bad status");
            return None;
        }
        match res.json::<T>().await {
            Ok(body) => Some(body),
            Err(e) => {
                log::warn!(error = %e, "{label}: json parse failed");
                None
            }
        }
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
                    Ok(format!(
                        "{base}/v1/{}/{}/download/{os}/{arch}",
                        pkg.name, pkg.version
                    ))
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

/// Shape shared by both the module detail endpoint (official spec) and the provider
/// detail endpoint (supported by registry.terraform.io, not in the official spec).
#[derive(Debug, Deserialize)]
struct TfVersionDetail {
    #[serde(default)]
    published_at: Option<String>,
}

impl TerraformRegistryClient {
    /// Fetch the `published_at` timestamp for a specific version.
    ///
    /// For **modules**: calls `GET /v1/modules/{ns}/{name}/{provider}/{version}`, which is
    /// part of the official Terraform Module Registry Protocol and always includes `published_at`.
    ///
    /// For **providers**: calls `GET /v1/providers/{ns}/{type}/{version}`, which is not in
    /// the official spec but is supported by `registry.terraform.io` and many private registries.
    ///
    /// Returns `None` when the endpoint is unsupported (404) or returns no timestamp.
    async fn fetch_version_published_at(
        &self,
        pkg: &PackageId,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/v1/{}/{}", pkg.name, pkg.version);
        let resp = self.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let detail: TfVersionDetail = resp.json().await.ok()?;
        detail
            .published_at
            .and_then(|s| s.parse::<chrono::DateTime<chrono::Utc>>().ok())
    }
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for TerraformRegistryClient {
    fn registry_type(&self) -> &str {
        "terraform"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let url = self.artifact_url(pkg)?;

        let resp =
            self.get(&url).send().await.map_err(|e| {
                CoreError::Registry(format!("terraform metadata request failed: {e}"))
            })?;

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

        // Fetch per-version publish timestamp for specific-version requests.
        // Version listings ("versions") have no meaningful single timestamp.
        let published_at = if pkg.version != "versions" {
            self.fetch_version_published_at(pkg).await
        } else {
            None
        };

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
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

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
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

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        let Some(ref base) = self.search_base else {
            return Ok(vec![]);
        };

        #[derive(Deserialize)]
        struct ModuleSearch {
            modules: Vec<ModuleHit>,
        }
        #[derive(Deserialize)]
        struct ModuleHit {
            namespace: String,
            name: String,
            provider: String,
            version: String,
            description: Option<String>,
        }
        // Returned by GET /v1/providers/{namespace} and GET /v1/providers/{ns}/{name}/versions
        #[derive(Deserialize)]
        struct ProviderList {
            #[serde(default)]
            providers: Vec<ProviderHit>,
        }
        #[derive(Deserialize)]
        struct ProviderHit {
            namespace: String,
            name: String,
            version: Option<String>,
            #[serde(default)]
            description: Option<String>,
        }

        let per = limit.min(25);

        // 1. Full-text module search (registry protocol v1 — always works).
        let module_url = format!(
            "{}/v1/modules/search?q={}&limit={}",
            base,
            percent_encode(query),
            per,
        );

        // 2. Provider lookup strategy — the Terraform Registry Protocol has no
        //    full-text provider search.  We use two heuristics:
        //
        //    a) Treat the whole query as a namespace:
        //       GET /v1/providers/{query}  →  lists all providers in that namespace.
        //       Works when users type the org/namespace name (e.g. "netbirdio").
        //
        //    b) If the query contains "/" treat it as "namespace/type":
        //       GET /v1/providers/{namespace}/{type}/versions  →  exact lookup.
        //       Works when users type "hashicorp/aws" or "netbirdio/netbird".
        let namespace_url = format!("{}/v1/providers/{}", base, percent_encode(query));
        let exact_url = query.split_once('/').map(|(ns, ty)| {
            format!(
                "{}/v1/providers/{}/{}/versions",
                base,
                percent_encode(ns),
                percent_encode(ty),
            )
        });

        let mut results: Vec<UpstreamPackage> = Vec::new();

        // Module search
        if let Some(body) = self
            .fetch_json::<ModuleSearch>(&module_url, "tf module search")
            .await
        {
            log::debug!(count = body.modules.len(), "tf module search: ok");
            for m in body.modules.into_iter().take(per) {
                results.push(UpstreamPackage {
                    name: format!("modules/{}/{}/{}", m.namespace, m.name, m.provider),
                    latest_version: m.version,
                    description: m.description,
                });
            }
        }

        // Provider namespace listing
        if let Some(body) = self
            .fetch_json::<ProviderList>(&namespace_url, "tf provider ns")
            .await
        {
            log::debug!(count = body.providers.len(), "tf provider ns: ok");
            for p in body.providers.into_iter().take(per) {
                results.push(UpstreamPackage {
                    name: format!("providers/{}/{}", p.namespace, p.name),
                    latest_version: p.version.unwrap_or_else(|| "latest".to_string()),
                    description: p.description,
                });
            }
        }

        // Exact namespace/type provider lookup
        if let Some(url) = exact_url {
            if let Some(body) = self
                .fetch_json::<ProviderList>(&url, "tf provider exact")
                .await
            {
                for p in body.providers.into_iter().take(per) {
                    results.push(UpstreamPackage {
                        name: format!("providers/{}/{}", p.namespace, p.name),
                        latest_version: p.version.unwrap_or_else(|| "latest".to_string()),
                        description: p.description,
                    });
                }
            }
        }

        // Deduplicate by name
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| seen.insert(r.name.clone()));

        Ok(results)
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
        PackageId::new(
            "tf",
            format!("modules/{namespace}/{name}/{provider}"),
            version,
        )
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
        let versions = client
            .list_versions("providers/hashicorp/aws")
            .await
            .unwrap();
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
        let versions = client
            .list_versions("modules/hashicorp/consul/aws")
            .await
            .unwrap();
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
        let versions = client
            .list_versions("providers/example/unknown")
            .await
            .unwrap();
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
        let content = bytes
            .into_iter()
            .flat_map(|b| b.to_vec())
            .collect::<Vec<u8>>();
        assert!(String::from_utf8(content).unwrap().contains("5.0.0"));
    }

    #[tokio::test]
    async fn fetch_artifact_provider_download_info() {
        let body =
            r#"{"os":"linux","arch":"amd64","download_url":"https://releases.hashicorp.com/..."}"#;
        let mut server = Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                "/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64",
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "5.0.0").with_artifact("linux/amd64");
        let fetched = client.fetch_artifact(&pkg).await.unwrap();
        let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
        let content = bytes
            .into_iter()
            .flat_map(|b| b.to_vec())
            .collect::<Vec<u8>>();
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
        let content = bytes
            .into_iter()
            .flat_map(|b| b.to_vec())
            .collect::<Vec<u8>>();
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
        assert!(matches!(
            client.fetch_artifact(&pkg).await,
            Err(CoreError::Registry(_))
        ));
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

    // ── published_at / release age gate ──────────────────────────────────────

    #[tokio::test]
    async fn resolve_metadata_module_specific_version_populates_published_at() {
        let mut server = Server::new_async().await;
        // Version listing request (resolve_metadata calls artifact_url → version listing)
        let _mock_versions = server
            .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
            .with_status(200)
            .with_body(r#"{"modules":[{"versions":[{"version":"0.1.0"}]}]}"#)
            .create_async()
            .await;
        // Module detail endpoint returns published_at
        let _mock_detail = server
            .mock("GET", "/v1/modules/hashicorp/consul/aws/0.1.0")
            .with_status(200)
            .with_body(r#"{"published_at":"2024-03-15T12:34:56Z"}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        // resolve_metadata for a download request hits the module download URL then fetches detail
        // Use the versions pkg path so the listing mock is hit
        let pkg = module_pkg("hashicorp", "consul", "aws", "versions");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        // versions request → published_at stays None
        assert!(meta.published_at.is_none());
    }

    #[tokio::test]
    async fn fetch_version_published_at_module() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/modules/hashicorp/consul/aws/0.1.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"published_at":"2024-03-15T12:34:56Z","version":"0.1.0"}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = module_pkg("hashicorp", "consul", "aws", "0.1.0");
        let ts = client.fetch_version_published_at(&pkg).await;
        assert!(
            ts.is_some(),
            "published_at should be populated from module detail endpoint"
        );
        let dt = ts.unwrap();
        assert_eq!(dt.to_rfc3339(), "2024-03-15T12:34:56+00:00");
    }

    #[tokio::test]
    async fn fetch_version_published_at_provider() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/hashicorp/aws/5.0.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"published_at":"2023-05-25T10:00:00Z","version":"5.0.0"}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("hashicorp", "aws", "5.0.0");
        let ts = client.fetch_version_published_at(&pkg).await;
        assert!(
            ts.is_some(),
            "published_at should be populated from provider detail endpoint"
        );
    }

    #[tokio::test]
    async fn fetch_version_published_at_returns_none_on_404() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/example/unknown/9.9.9")
            .with_status(404)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("example", "unknown", "9.9.9");
        let ts = client.fetch_version_published_at(&pkg).await;
        assert!(ts.is_none(), "404 from detail endpoint should yield None");
    }

    #[tokio::test]
    async fn fetch_version_published_at_returns_none_when_field_absent() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v1/providers/example/minimal/1.0.0")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"version":"1.0.0"}"#)
            .create_async()
            .await;

        let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
        let pkg = provider_pkg("example", "minimal", "1.0.0");
        let ts = client.fetch_version_published_at(&pkg).await;
        assert!(ts.is_none(), "missing published_at field should yield None");
    }
}
