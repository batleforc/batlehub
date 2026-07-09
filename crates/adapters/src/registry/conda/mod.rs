use serde::Deserialize;

use batlehub_core::{entities::PackageId, error::CoreError};

use super::http_client::{basic_auth_get, new_http_client, UpstreamHttpOptions};

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
    pub(super) list_platforms: Vec<String>,
}

impl CondaRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> Result<Self, CoreError> {
        let http = new_http_client(None, opts)?;
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

    pub(super) fn artifact_url(&self, pkg: &PackageId) -> String {
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

#[cfg(test)]
mod tests;
