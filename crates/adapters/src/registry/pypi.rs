use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};
// percent_encode not needed for PyPI (uses normalize_name for exact lookup)

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
/// - `artifact`: filename of the specific distribution file (e.g. `"requests-2.28.0-py3-none-any.whl"`)
///   When `None`, `resolve_metadata` returns metadata without a specific artifact URL.
pub struct PypiRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
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

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }
}

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

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PypiVersionJson {
    #[serde(default)]
    urls: Vec<PypiFileInfo>,
}

#[derive(Debug, Deserialize)]
struct PypiPackageJson {
    #[serde(default)]
    releases: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PypiFileInfo {
    filename: String,
    url: String,
    #[serde(default)]
    digests: PypiDigests,
    #[serde(default)]
    upload_time_iso_8601: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct PypiDigests {
    #[serde(default)]
    sha256: Option<String>,
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

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

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

        let cache_control = dl_resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

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
        #[derive(Deserialize)]
        struct PypiInfo {
            info: PypiInfoInner,
        }
        #[derive(Deserialize)]
        struct PypiInfoInner {
            name: String,
            version: String,
            summary: Option<String>,
        }

        let base = self.base_url.trim_end_matches('/');
        // PyPI removed its public search API; do an exact name lookup instead.
        let url = format!("{base}/pypi/{}/json", normalize_name(query));
        let res = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: PypiInfo = res
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

// ── Proxy helpers used by the HTTP handler layer ──────────────────────────────

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

    let body = resp
        .bytes()
        .await
        .map_err(|e| CoreError::Registry(e.to_string()))?;

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

fn rewrite_simple_html(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return body.to_vec(),
    };

    // Replace href="https://files.pythonhosted.org/packages/.../filename.ext#sha=..."
    // with href="/proxy/{registry}/packages/filename.ext#sha=..."
    // We need to keep the fragment (#sha256=...) but rewrite the base URL.
    let proxy_packages = format!("{proxy_base}/proxy/{registry}/packages");
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(href_pos) = remaining.find("href=\"") {
        let after_quote = &remaining[href_pos + 6..];
        result.push_str(&remaining[..href_pos + 6]);

        if let Some(end_quote) = after_quote.find('"') {
            let href_value = &after_quote[..end_quote];
            remaining = &after_quote[end_quote..];

            // Check if this looks like a PyPI CDN URL or any absolute URL to a file
            if href_value.starts_with("https://") || href_value.starts_with("http://") {
                // Extract just the filename (last path segment) and fragment
                if let Some(fragment_pos) = href_value.rfind('#') {
                    let path_part = &href_value[..fragment_pos];
                    let fragment = &href_value[fragment_pos..];
                    if let Some(slash_pos) = path_part.rfind('/') {
                        let filename = &path_part[slash_pos + 1..];
                        result.push_str(&format!("{proxy_packages}/{filename}{fragment}"));
                        continue;
                    }
                } else if let Some(slash_pos) = href_value.rfind('/') {
                    let filename = &href_value[slash_pos + 1..];
                    result.push_str(&format!("{proxy_packages}/{filename}"));
                    continue;
                }
            }
            result.push_str(href_value);
        } else {
            remaining = after_quote;
        }
    }
    result.push_str(remaining);
    result.into_bytes()
}

fn rewrite_simple_json(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_normalization() {
        assert_eq!(normalize_name("Pillow"), "pillow");
        assert_eq!(normalize_name("my_pkg"), "my-pkg");
        assert_eq!(normalize_name("A.B.C"), "a-b-c");
        assert_eq!(normalize_name("My--Package"), "my-package");
        assert_eq!(normalize_name("requests"), "requests");
    }

    #[test]
    fn rewrite_simple_html_rewrites_cdn_urls() {
        let html = br#"<a href="https://files.pythonhosted.org/packages/ab/cd/requests-2.28.0.tar.gz#sha256=abc">requests-2.28.0.tar.gz</a>"#;
        let out = rewrite_simple_html(html, "my-pypi", "http://localhost:8080");
        let out_str = std::str::from_utf8(&out).unwrap();
        assert!(out_str.contains("/proxy/my-pypi/packages/requests-2.28.0.tar.gz#sha256=abc"));
        assert!(!out_str.contains("files.pythonhosted.org"));
    }

    #[test]
    fn rewrite_simple_html_keeps_relative_hrefs() {
        let html = br#"<a href="/simple/">index</a>"#;
        let out = rewrite_simple_html(html, "my-pypi", "http://localhost:8080");
        let out_str = std::str::from_utf8(&out).unwrap();
        assert!(out_str.contains(r#"href="/simple/""#));
    }

    #[test]
    fn rewrite_simple_json_rewrites_urls() {
        let json = serde_json::json!({
            "files": [
                { "filename": "foo-1.0.whl", "url": "https://files.pythonhosted.org/packages/xx/foo-1.0.whl#sha256=deadbeef" }
            ]
        });
        let body = serde_json::to_vec(&json).unwrap();
        let out = rewrite_simple_json(&body, "my-pypi", "http://localhost");
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        let url = parsed["files"][0]["url"].as_str().unwrap();
        assert_eq!(
            url,
            "http://localhost/proxy/my-pypi/packages/foo-1.0.whl#sha256=deadbeef"
        );
    }

    #[tokio::test]
    async fn resolve_metadata_finds_wheel() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/pypi/requests/2.28.0/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(serde_json::to_string(&serde_json::json!({
                "urls": [
                    {
                        "filename": "requests-2.28.0-py3-none-any.whl",
                        "url": "https://files.pythonhosted.org/packages/requests-2.28.0-py3-none-any.whl",
                        "digests": { "sha256": "abc123" },
                        "upload_time_iso_8601": "2022-10-26T18:17:01.491020Z"
                    }
                ]
            })).unwrap())
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = PypiRegistryClient::new(server.url(), &opts).unwrap();

        let pkg = PackageId::new("my-pypi", "requests", "2.28.0")
            .with_artifact("requests-2.28.0-py3-none-any.whl");
        let meta = client.resolve_metadata(&pkg).await.unwrap();

        assert_eq!(meta.checksum.as_deref(), Some("abc123"));
        assert!(meta.download_url.is_some());
        assert!(meta.published_at.is_some());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn resolve_metadata_404_returns_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/pypi/nonexistent/1.0.0/json")
            .with_status(404)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = PypiRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = PackageId::new("reg", "nonexistent", "1.0.0");
        let err = client.resolve_metadata(&pkg).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_versions_parses_releases() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/pypi/requests/json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::to_string(&serde_json::json!({
                    "releases": {
                        "2.27.0": [],
                        "2.28.0": [],
                        "2.28.1": []
                    }
                }))
                .unwrap(),
            )
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = PypiRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("requests").await.unwrap();
        assert_eq!(versions, vec!["2.27.0", "2.28.0", "2.28.1"]);
    }
}
