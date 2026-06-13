use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{
    apply_upstream_options, basic_auth_get, cache_control, UpstreamHttpOptions,
};

mod client;
mod models;

#[cfg(feature = "local-registry")]
pub use client::parse_conda_metadata;
pub use models::CondaPackageInfo;

/// Conda channel proxy client.
///
/// Proxies a single conda channel (e.g. `conda-forge`) across all platforms.
///
/// Default upstream: `https://conda.anaconda.org`
///
/// `PackageId` conventions (repodata):
/// - `name`: `"repodata"` for the channel index, or the package filename stem
/// - `version`: platform string (e.g. `"linux-64"`, `"noarch"`)
/// - `artifact`: `None` for repodata, `Some("<filename>")` for a specific package
///
/// `list_versions` is implemented by fetching `repodata.json` for each of the
/// `list_platforms` (default: `noarch` + the four major binary platforms) and
/// collecting every distinct version string for the named package.
pub struct CondaRegistryClient {
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
    /// Platforms queried when `list_versions` is called.
    /// Defaults to the five most common platforms.
    list_platforms: Vec<String>,
}

impl CondaRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().user_agent("batlehub/0.1");
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
            list_platforms: default_list_platforms(),
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    fn artifact_url(&self, pkg: &PackageId) -> String {
        let base = self.base_url.trim_end_matches('/');
        let platform = &pkg.version; // version = platform for conda

        match pkg.artifact.as_deref() {
            None | Some("repodata.json") => {
                format!("{base}/{platform}/repodata.json")
            }
            Some("current_repodata.json") => {
                format!("{base}/{platform}/current_repodata.json")
            }
            Some(filename) => {
                format!("{base}/{platform}/{filename}")
            }
        }
    }
}

/// The five platforms queried by `list_versions` to synthesise a version list
/// from `repodata.json`.  `noarch` covers pure-Python and architecture-neutral
/// packages and is tried first because it is the smallest repodata file.
fn default_list_platforms() -> Vec<String> {
    ["noarch", "linux-64", "osx-64", "osx-arm64", "win-64"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for CondaRegistryClient {
    fn registry_type(&self) -> &str {
        "conda"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let platform = &pkg.version;

        // For specific package files, look them up in repodata.json.
        if pkg.name != "repodata" {
            if let Some(filename) = &pkg.artifact {
                return self
                    .lookup_file_in_repodata(base, platform, filename, pkg)
                    .await;
            }
        }

        // For repodata.json itself, return the URL as the download URL.
        let url = self.artifact_url(pkg);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("conda metadata request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "conda resource not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "conda upstream returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

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
        let url = self.artifact_url(pkg);

        tracing::debug!(url = %url, "fetching conda artifact");

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "conda artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "conda upstream returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

        let stream = resp
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    /// Synthesise a version list by scanning `repodata.json` for each of the
    /// configured `list_platforms`.  Platforms that return a 404 or network
    /// error are silently skipped so a missing platform never blocks warming.
    /// Versions are collected into a sorted, deduplicated list.
    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let mut versions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for platform in &self.list_platforms {
            let platform_versions = self.fetch_platform_versions(base, platform, package).await;
            versions.extend(platform_versions);
        }

        Ok(versions.into_iter().collect())
    }
}

#[cfg(test)]
mod tests;
