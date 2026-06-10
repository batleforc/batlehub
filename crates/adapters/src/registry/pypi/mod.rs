use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{
    apply_upstream_options, basic_auth_get, cache_control, UpstreamHttpOptions,
};
// percent_encode not needed for PyPI (uses normalize_name for exact lookup)

mod client;
mod models;

pub use client::{fetch_simple_page, normalize_name, rewrite_simple_page};
use models::{PypiPackageJson, PypiSearchInfo, PypiVersionJson};

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

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for PypiRegistryClient {
    fn registry_type(&self) -> &str {
        "pypi"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let name = normalize_name(&pkg.name);
        let url = format!("{base}/pypi/{name}/{}/json", pkg.version);

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("pypi metadata request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "pypi package not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi upstream returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

        let body = resp
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let version_json: PypiVersionJson = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Registry(format!("pypi: parse version JSON: {e}")))?;

        // Find the specific file matching pkg.artifact, or use the first file.
        let file = match &pkg.artifact {
            Some(filename) => version_json
                .urls
                .into_iter()
                .find(|f| f.filename == *filename),
            None => version_json.urls.into_iter().next(),
        };

        let (download_url, checksum, published_at) = match file {
            Some(f) => {
                let published_at = f.upload_time_iso_8601.as_deref().and_then(|s| {
                    chrono::DateTime::parse_from_rfc3339(s)
                        .ok()
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                });
                (Some(f.url), f.digests.sha256, published_at)
            }
            None => (None, None, None),
        };

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
            download_url,
            checksum,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let name = normalize_name(&pkg.name);
        let version = &pkg.version;

        // Resolve the download URL from the JSON API, then stream from the CDN.
        let api_url = format!("{base}/pypi/{name}/{version}/json");
        let artifact_filename = pkg.artifact.as_deref().unwrap_or("");

        let api_resp = self
            .get(&api_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("pypi: API request failed: {e}")))?;

        if api_resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "pypi artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !api_resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi upstream returned {} for {}",
                api_resp.status(),
                pkg.cache_key()
            )));
        }

        let body = api_resp
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let version_json: PypiVersionJson = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Registry(format!("pypi: parse version JSON: {e}")))?;

        let file = version_json
            .urls
            .into_iter()
            .find(|f| f.filename == artifact_filename)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "pypi: file '{}' not found in version {}",
                    artifact_filename, version
                ))
            })?;

        tracing::debug!(url = %file.url, "fetching PyPI artifact");

        let dl_resp = self
            .get(&file.url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !dl_resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi CDN returned {} for {}",
                dl_resp.status(),
                artifact_filename
            )));
        }

        let cache_control = cache_control(&dl_resp);

        let stream = dl_resp
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let name = normalize_name(package);
        let url = format!("{base}/pypi/{name}/json");

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi upstream returned {} listing versions for {name}",
                resp.status()
            )));
        }

        let body = resp
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let pkg_json: PypiPackageJson = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Registry(format!("pypi: parse package JSON: {e}")))?;

        let mut versions: Vec<String> = pkg_json.releases.into_keys().collect();
        versions.sort();
        Ok(versions)
    }

    // PyPI removed its public search XMLRPC endpoint. Fall back to exact name
    // lookup: if the query exactly matches a published package, return it.
    async fn search_packages(
        &self,
        query: &str,
        _limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/pypi/{}/json", normalize_name(query));
        let res = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: PypiSearchInfo = res
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(vec![UpstreamPackage {
            name: body.info.name,
            latest_version: body.info.version,
            description: body.info.summary,
        }])
    }
}

#[cfg(test)]
mod tests;
