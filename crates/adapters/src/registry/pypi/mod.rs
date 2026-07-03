use serde::Deserialize;

use super::http_client::{apply_upstream_options, basic_auth_get, UpstreamHttpOptions};

mod client;
mod models;

pub use client::{fetch_simple_page, normalize_name, rewrite_simple_page};

/// PyPI registry proxy client.
///
/// Implements the PyPI JSON API and Simple Repository API (PEP 503/691).
///
/// Default upstream: `https://pypi.org`
///
/// `PackageId` conventions:
/// - `name`: PEP 503-normalised package name (lower-case, `[-_.]` → `-`)
/// - `version`:
///   - a version string (e.g. `"2.28.0"`) → `GET /pypi/{name}/{version}/json`
///   - `"__all__"` → `GET /pypi/{name}/json` (all versions, for `list_versions`)
/// - `artifact`: filename of the specific distribution file
///   When `None`, `resolve_metadata` returns metadata without a specific artifact URL.
pub struct PypiRegistryClient {
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
}

impl PypiRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().user_agent("batlehub/0.1");
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }
}

#[cfg(test)]
mod tests;
