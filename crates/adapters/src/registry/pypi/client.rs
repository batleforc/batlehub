use async_trait::async_trait;
use futures::TryStreamExt;

use super::super::http_client::{cache_control, to_registry_error};
use super::models::{PypiPackageJson, PypiSearchInfo, PypiVersionJson};
use super::PypiRegistryClient;
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

// ── PEP 503 name normalisation ────────────────────────────────────────────────

/// Normalise a PyPI package name per PEP 503: lower-case, collapse runs of
/// `[-_.]` into a single `-`.
pub fn normalize_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut result = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch == '-' || ch == '_' || ch == '.' {
            if !prev_dash {
                result.push('-');
                prev_dash = true;
            }
        } else {
            result.push(ch);
            prev_dash = false;
        }
    }
    result
}

/// Fetch the Simple API HTML (or JSON) page for a package from the upstream.
///
/// Returns the raw body bytes and the `Content-Type` header value so the
/// handler can forward it to the client after URL rewriting.
pub async fn fetch_simple_page(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    basic_auth: Option<&(String, String)>,
    accept: Option<&str>,
) -> Result<(bytes::Bytes, Option<String>), CoreError> {
    let normalized = normalize_name(name);
    let url = format!("{}/simple/{}/", base_url.trim_end_matches('/'), normalized);

    let mut builder = client.get(&url);
    if let Some((u, p)) = basic_auth {
        builder = builder.basic_auth(u, Some(p));
    }
    if let Some(accept_val) = accept {
        builder = builder.header(reqwest::header::ACCEPT, accept_val);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| CoreError::Registry(format!("pypi: simple page request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CoreError::NotFound(format!(
            "pypi: package '{}' not found in simple index",
            name
        )));
    }
    if !resp.status().is_success() {
        return Err(CoreError::Registry(format!(
            "pypi: simple index returned {}",
            resp.status()
        )));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let body = resp.bytes().await.map_err(to_registry_error)?;

    Ok((body, content_type))
}

/// Rewrite href/url values in a PyPI simple page so all file links go through
/// the batlehub proxy at `/proxy/{registry}/packages/{filename}`.
///
/// Handles both HTML (PEP 503) and JSON (PEP 691) formats.
pub fn rewrite_simple_page(
    body: &[u8],
    content_type: Option<&str>,
    registry: &str,
    proxy_base: &str,
) -> Vec<u8> {
    let is_json = content_type
        .map(|ct| ct.contains("application/vnd.pypi.simple"))
        .unwrap_or(false);

    if is_json {
        rewrite_simple_json(body, registry, proxy_base)
    } else {
        rewrite_simple_html(body, registry, proxy_base)
    }
}

/// Rewrite one `href` value if it is an absolute HTTP URL pointing to a PyPI
/// CDN file. Returns `Some(rewritten)` when rewriting is applicable, `None`
/// when the original value should be kept unchanged.
fn rewrite_abs_href(href_value: &str, proxy_packages: &str) -> Option<String> {
    if !href_value.starts_with("https://") && !href_value.starts_with("http://") {
        return None;
    }
    if let Some(fragment_pos) = href_value.rfind('#') {
        let path_part = &href_value[..fragment_pos];
        let fragment = &href_value[fragment_pos..];
        if let Some(slash_pos) = path_part.rfind('/') {
            let filename = &path_part[slash_pos + 1..];
            return Some(format!("{proxy_packages}/{filename}{fragment}"));
        }
    } else if let Some(slash_pos) = href_value.rfind('/') {
        let filename = &href_value[slash_pos + 1..];
        return Some(format!("{proxy_packages}/{filename}"));
    }
    None
}

pub(super) fn rewrite_simple_html(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return body.to_vec(),
    };

    let proxy_packages = format!("{proxy_base}/proxy/{registry}/packages");
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(href_pos) = remaining.find("href=\"") {
        let after_quote = &remaining[href_pos + 6..];
        result.push_str(&remaining[..href_pos + 6]);

        if let Some(end_quote) = after_quote.find('"') {
            let href_value = &after_quote[..end_quote];
            remaining = &after_quote[end_quote..];
            let rewritten = rewrite_abs_href(href_value, &proxy_packages)
                .unwrap_or_else(|| href_value.to_owned());
            result.push_str(&rewritten);
        } else {
            remaining = after_quote;
        }
    }
    result.push_str(remaining);
    result.into_bytes()
}

pub(super) fn rewrite_simple_json(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
    let mut json: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };

    let proxy_packages = format!("{proxy_base}/proxy/{registry}/packages");

    if let Some(files) = json.get_mut("files").and_then(|f| f.as_array_mut()) {
        for file in files.iter_mut() {
            if let Some(url_val) = file.get_mut("url") {
                if let Some(url_str) = url_val.as_str() {
                    let rewritten = rewrite_file_url(url_str, &proxy_packages);
                    *url_val = serde_json::Value::String(rewritten);
                }
            }
        }
    }

    serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec())
}

fn rewrite_file_url(url: &str, proxy_packages: &str) -> String {
    // Split off fragment first
    let (path_part, fragment) = if let Some(frag_pos) = url.rfind('#') {
        (&url[..frag_pos], &url[frag_pos..])
    } else {
        (url, "")
    };

    if let Some(slash_pos) = path_part.rfind('/') {
        let filename = &path_part[slash_pos + 1..];
        format!("{proxy_packages}/{filename}{fragment}")
    } else {
        url.to_owned()
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

        let body = resp.bytes().await.map_err(to_registry_error)?;

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

        let body = api_resp.bytes().await.map_err(to_registry_error)?;

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
            .map_err(to_registry_error)?;

        if !dl_resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi CDN returned {} for {}",
                dl_resp.status(),
                artifact_filename
            )));
        }

        let cache_control = cache_control(&dl_resp);

        let stream = dl_resp.bytes_stream().map_err(to_registry_error);

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let name = normalize_name(package);
        let url = format!("{base}/pypi/{name}/json");

        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "pypi upstream returned {} listing versions for {name}",
                resp.status()
            )));
        }

        let body = resp.bytes().await.map_err(to_registry_error)?;

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
        let res = self.get(&url).send().await.map_err(to_registry_error)?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: PypiSearchInfo = res.json().await.map_err(to_registry_error)?;

        Ok(vec![UpstreamPackage {
            name: body.info.name,
            latest_version: body.info.version,
            description: body.info.summary,
        }])
    }
}
