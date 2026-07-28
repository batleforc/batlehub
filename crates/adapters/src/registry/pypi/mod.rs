use batlehub_core::error::CoreError;
use serde::Deserialize;

use super::http_client::{
    apply_upstream_tls, basic_auth_get, new_http_client, same_origin, UpstreamHttpOptions,
};

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
    /// Credential-free client (same TLS/proxy settings, but no bearer/custom-header
    /// auth baked in). Used to fetch cross-origin download URLs so configured
    /// credentials never leave the index origin.
    pub(super) http_plain: reqwest::Client,
    pub(super) base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
}

impl PypiRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> Result<Self, CoreError> {
        let http = new_http_client(None, opts)?;
        // Same TLS/proxy/timeout settings, but WITHOUT auth headers.
        let http_plain =
            apply_upstream_tls(reqwest::Client::builder().user_agent("batlehub/0.1"), opts)
                .map_err(CoreError::Other)?
                .build()
                .map_err(|e| CoreError::Other(e.into()))?;
        Ok(Self {
            http,
            http_plain,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        basic_auth_get(&self.http, &self.basic_auth, url)
    }

    /// Build a GET for an artifact download URL taken from the index JSON.
    ///
    /// PyPI legitimately serves files from a different origin than the index
    /// (pypi.org → files.pythonhosted.org), so we cannot require same-origin.
    /// But the `url` is index-controlled: a malicious or compromised index could
    /// point it at an internal address or attacker host. Attach configured
    /// credentials (Basic + any bearer/custom default headers) ONLY when the URL
    /// shares the index origin; otherwise fetch with the credential-free client
    /// so the operator's index credentials never leak off-origin.
    pub(super) fn get_download(&self, url: &str) -> reqwest::RequestBuilder {
        let same = match (
            reqwest::Url::parse(url),
            reqwest::Url::parse(&self.base_url),
        ) {
            (Ok(u), Ok(base)) => same_origin(&u, &base),
            // Unparseable URL → treat as cross-origin (fail safe: no credentials).
            _ => false,
        };
        if same {
            self.get(url)
        } else {
            self.http_plain.get(url)
        }
    }
}

#[cfg(test)]
mod tests;
