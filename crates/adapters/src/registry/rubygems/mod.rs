use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{
    apply_upstream_options, basic_auth_get, cache_control, percent_encode, UpstreamHttpOptions,
};

mod client;
mod models;

#[cfg(feature = "local-registry")]
pub use client::parse_gem_bytes;
pub use client::split_gem_stem;
pub use models::GemMetadata;

use models::{GemInfo, GemVersion};

/// RubyGems registry proxy client.
///
/// Implements the RubyGems REST API v1:
/// <https://guides.rubygems.org/rubygems-org-api/>
///
/// Default upstream: `https://rubygems.org`
///
/// `PackageId` conventions:
/// - `name`: gem name (e.g. `"rails"`) or `"_index"` for index files
/// - `version`:
///   - `"info"` → `/api/v1/gems/{name}.json`
///   - `"versions"` → `/api/v1/versions/{name}.json`
///   - `"specs"` → `/specs.4.8.gz`
///   - `"latest_specs"` → `/latest_specs.4.8.gz`
///   - `"prerelease_specs"` → `/prerelease_specs.4.8.gz`
///   - a semver string (with `artifact` set) → versioned gem resource
/// - `artifact`:
///   - `Some("gem")` → `/gems/{name}-{version}.gem`
///   - `Some("gemspec")` → `/quick/Marshal.4.8/{name}-{version}.gemspec.rz`
///   - `None` → REST endpoint determined by `version` field
pub struct RubyGemsRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl RubyGemsRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().user_agent("batlehub/0.1");
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
        })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    fn artifact_url(&self, pkg: &PackageId) -> Result<String, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let name = &pkg.name;
        let version = &pkg.version;

        if name == "_index" {
            return match version.as_str() {
                "specs" => Ok(format!("{base}/specs.4.8.gz")),
                "latest_specs" => Ok(format!("{base}/latest_specs.4.8.gz")),
                "prerelease_specs" => Ok(format!("{base}/prerelease_specs.4.8.gz")),
                other => Err(CoreError::Registry(format!(
                    "rubygems: unknown index variant '{other}'"
                ))),
            };
        }

        match pkg.artifact.as_deref() {
            Some("gem") => Ok(format!("{base}/gems/{name}-{version}.gem")),
            Some("gemspec") => Ok(format!(
                "{base}/quick/Marshal.4.8/{name}-{version}.gemspec.rz"
            )),
            None => match version.as_str() {
                "versions" => Ok(format!("{base}/api/v1/versions/{name}.json")),
                _ => Ok(format!("{base}/api/v1/gems/{name}.json")),
            },
            Some(other) => Err(CoreError::Registry(format!(
                "rubygems: unknown artifact type '{other}'"
            ))),
        }
    }
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for RubyGemsRegistryClient {
    fn registry_type(&self) -> &str {
        "rubygems"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let url = self.artifact_url(pkg)?;

        let resp =
            self.get(&url).send().await.map_err(|e| {
                CoreError::Registry(format!("rubygems metadata request failed: {e}"))
            })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "rubygems resource not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "rubygems upstream returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

        // Parse published_at and checksum from the gem info JSON when available.
        if pkg.artifact.is_none() && pkg.name != "_index" {
            let body = resp
                .bytes()
                .await
                .map_err(|e| CoreError::Registry(e.to_string()))?;
            if let Ok(info) = serde_json::from_slice::<GemInfo>(&body) {
                let published_at = info.created_at.as_deref().and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });
                return Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at,
                    download_url: Some(url),
                    checksum: info.sha,
                    is_signed: None,
                    extra: serde_json::json!({ "version": info.version }),
                    cache_control,
                });
            }
        }

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

        tracing::debug!(url = %url, "fetching RubyGems artifact");

        let response = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "rubygems artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !response.status().is_success() {
            return Err(CoreError::Registry(format!(
                "rubygems upstream returned {} for {}",
                response.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&response);

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
        let url = format!("{base}/api/v1/versions/{package}.json");

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

        let versions: Vec<GemVersion> = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Registry(format!("rubygems: parse versions: {e}")))?;

        // rubygems API returns newest-first; reverse to oldest-first.
        let mut result: Vec<String> = versions.into_iter().map(|v| v.number).collect();
        result.reverse();
        Ok(result)
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(Deserialize)]
        struct GemSearchResult {
            name: String,
            version: String,
            info: Option<String>,
        }

        let url = format!(
            "{}/api/v1/search.json?query={}&page=1",
            self.base_url,
            percent_encode(query),
        );
        let res = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let gems: Vec<GemSearchResult> = res
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(gems
            .into_iter()
            .take(limit)
            .map(|g| UpstreamPackage {
                name: g.name,
                latest_version: g.version,
                description: g.info,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
