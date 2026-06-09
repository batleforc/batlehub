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

mod models;
mod modules;
mod providers;

/// Terraform provider and module registry proxy client.
///
/// Implements the Terraform Registry Protocol v1:
/// - Providers: <https://developer.hashicorp.com/terraform/internals/provider-registry-protocol>
/// - Modules:   <https://developer.hashicorp.com/terraform/internals/module-registry-protocol>
///
/// Default upstream: `https://registry.terraform.io`
///
/// `PackageId.name`: `"providers/{ns}/{type}"` or `"modules/{ns}/{name}/{provider}"`.
/// `PackageId.version`: `"versions"` for listing or a semver string.
/// `PackageId.artifact`: `None` (listing), `"{os}/{arch}"` (provider download), `"download"` (module).
pub struct TerraformRegistryClient {
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
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
    pub(super) async fn fetch_json<T: serde::de::DeserializeOwned>(
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

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
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
            modules::fetch_version_published_at(self, pkg).await
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

        modules::parse_versions(package, &body)
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        let Some(ref base) = self.search_base else {
            return Ok(vec![]);
        };

        let per = limit.min(25);

        // 1. Full-text module search (registry protocol v1 — always works).
        // 2. Provider lookup strategy — the Terraform Registry Protocol has no
        //    full-text provider search. Uses two heuristics (namespace + exact).
        let mut results: Vec<UpstreamPackage> = Vec::new();

        results.extend(providers::search_modules(self, base, query, per).await);
        results.extend(providers::search_providers(self, base, query, per).await);

        // Deduplicate by name
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| seen.insert(r.name.clone()));

        Ok(results)
    }
}

#[cfg(test)]
mod tests;
