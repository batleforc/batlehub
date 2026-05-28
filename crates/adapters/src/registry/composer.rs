use async_trait::async_trait;
use bytes::Bytes;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// Composer / Packagist registry proxy client.
///
/// Implements the Packagist v2 API:
/// <https://packagist.org/apidoc>
///
/// Default upstream: `https://repo.packagist.org`
///
/// `PackageId` conventions:
/// - `name`: `vendor/package` (e.g. `symfony/console`)
/// - `version`: semver string (e.g. `v7.2.0`) or `dev-*` branch alias
/// - `artifact`:
///   - `Some("dist")`     → stream the dist ZIP for that version
///   - `Some("p2")`       → fetch `p2/{vendor}/{package}.json` bytes (all versions)
///   - `Some("p2~dev")`   → fetch `p2/{vendor}/{package}~dev.json` bytes (dev variants)
///   - `None`             → version metadata only
pub struct ComposerRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl ComposerRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().user_agent("Composer/2.0 batlehub/0.1");
        let http = apply_upstream_options(builder, opts)?;
        // Normalise once so per-method callers don't need to trim.
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        Ok(Self { http, base_url, basic_auth: opts.basic_auth.clone() })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    /// Send a GET to the given p2 URL and check the status code.
    /// Returns the raw response on success; maps 404 to `CoreError::NotFound`.
    async fn send_p2_request(
        &self,
        url: &str,
        package: &str,
    ) -> Result<reqwest::Response, CoreError> {
        let resp = self
            .get(url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("composer p2 request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "composer package '{package}' not found"
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "composer upstream returned {} for '{package}'",
                resp.status()
            )));
        }
        Ok(resp)
    }

    async fn fetch_p2_response(
        &self,
        package: &str,
    ) -> Result<PackagistV2Response, CoreError> {
        let url = format!("{}/p2/{package}.json", self.base_url);
        let resp = self.send_p2_request(&url, package).await?;
        resp.json::<PackagistV2Response>()
            .await
            .map_err(|e| CoreError::Registry(format!("composer p2 parse error: {e}")))
    }

    /// Return the raw p2 JSON bytes for `package`, using `url_suffix` to select
    /// the exact endpoint (`""` for `.json`, `"~dev"` for `~dev.json`).
    async fn fetch_p2_bytes(
        &self,
        package: &str,
        url_suffix: &str,
    ) -> Result<(bytes::Bytes, Option<String>), CoreError> {
        let url = format!("{}/p2/{package}{url_suffix}.json", self.base_url);
        let resp = self.send_p2_request(&url, package).await?;

        let cache_control = resp
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let body = resp
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok((body, cache_control))
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct PackagistV2Response {
    packages: HashMap<String, Vec<ComposerVersionEntry>>,
}

#[derive(Debug, Deserialize, Clone)]
struct ComposerVersionEntry {
    version: String,
    dist: Option<ComposerDist>,
    time: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct ComposerDist {
    url: String,
    shasum: Option<String>,
    reference: Option<String>,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for ComposerRegistryClient {
    fn registry_type(&self) -> &str {
        "composer"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        // For p2 artifacts return synthetic metadata pointing to the upstream URL;
        // the actual bytes are fetched lazily by fetch_artifact.
        match pkg.artifact.as_deref() {
            Some(art @ ("p2" | "p2~dev")) => {
                let dev_suffix = if art == "p2~dev" { "~dev" } else { "" };
                let url = format!("{}/p2/{}{dev_suffix}.json", self.base_url, pkg.name);
                return Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: Some(url),
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control: None,
                });
            }
            _ => {}
        }

        let p2 = self.fetch_p2_response(&pkg.name).await?;
        let versions = p2.packages.get(&pkg.name).ok_or_else(|| {
            CoreError::NotFound(format!(
                "composer package '{}' not found in p2 response",
                pkg.name
            ))
        })?;

        let entry = versions
            .iter()
            .find(|v| v.version == pkg.version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "composer package '{}@{}' not found",
                    pkg.name, pkg.version
                ))
            })?;

        let download_url = if pkg.artifact.as_deref() == Some("dist") {
            entry.dist.as_ref().map(|d| d.url.clone())
        } else {
            None
        };

        let checksum = entry
            .dist
            .as_ref()
            .and_then(|d| d.shasum.as_deref())
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let published_at = entry
            .time
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let extra = serde_json::json!({
            "version": entry.version,
            "dist_url": entry.dist.as_ref().map(|d| &d.url),
            "dist_reference": entry.dist.as_ref().and_then(|d| d.reference.as_deref()),
        });

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
            download_url,
            checksum,
            is_signed: None,
            extra,
            cache_control: None,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        // For p2 and p2~dev artifacts, stream the raw JSON bytes.
        match pkg.artifact.as_deref() {
            Some(art @ ("p2" | "p2~dev")) => {
                let suffix = if art == "p2~dev" { "~dev" } else { "" };
                let (bytes, cache_control) = self.fetch_p2_bytes(&pkg.name, suffix).await?;
                let once = futures::stream::once(async move {
                    Ok::<bytes::Bytes, CoreError>(bytes)
                });
                return Ok(FetchedArtifact { stream: Box::pin(once), cache_control });
            }
            _ => {}
        }

        // For "dist" artifact, resolve and stream from the dist URL.
        let p2 = self.fetch_p2_response(&pkg.name).await?;
        let versions = p2.packages.get(&pkg.name).ok_or_else(|| {
            CoreError::NotFound(format!(
                "composer package '{}' not found in p2 response",
                pkg.name
            ))
        })?;

        let entry = versions
            .iter()
            .find(|v| v.version == pkg.version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "composer package '{}@{}' not found",
                    pkg.name, pkg.version
                ))
            })?;

        let dist_url = entry
            .dist
            .as_ref()
            .map(|d| d.url.clone())
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "no dist URL for composer package '{}@{}'",
                    pkg.name, pkg.version
                ))
            })?;

        tracing::debug!(url = %dist_url, "fetching composer dist artifact");

        let response = self
            .get(&dist_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "composer dist artifact not found: {}@{}",
                pkg.name, pkg.version
            )));
        }
        if !response.status().is_success() {
            return Err(CoreError::Registry(format!(
                "composer dist upstream returned {} for {}@{}",
                response.status(),
                pkg.name,
                pkg.version
            )));
        }

        let cache_control = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = response
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact { stream: Box::pin(stream), cache_control })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let p2 = self.fetch_p2_response(package).await?;
        let versions = p2
            .packages
            .get(package)
            .map(|entries| entries.iter().map(|e| e.version.clone()).collect())
            .unwrap_or_default();
        Ok(versions)
    }
}

// ── Publish helpers ───────────────────────────────────────────────────────────

/// Metadata extracted from a Composer package ZIP on publish.
pub struct ComposerPackageMeta {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub composer_json: serde_json::Value,
}

#[derive(Deserialize)]
struct ComposerJson {
    name: String,
    version: Option<String>,
    description: Option<String>,
}

/// Parse `composer.json` from the root of a Composer ZIP artifact.
///
/// The `version` field may be absent in `composer.json` (Packagist injects it),
/// so the caller may supply it via `version_override`.
pub fn parse_composer_zip(
    data: &Bytes,
    version_override: Option<&str>,
) -> anyhow::Result<ComposerPackageMeta> {
    use std::io::Cursor;

    let cursor = Cursor::new(data.as_ref());
    let mut archive = zip::ZipArchive::new(cursor)?;

    // composer.json may live at the root or in a single top-level directory.
    let json_content = find_composer_json(&mut archive)?;

    // Parse once into a generic Value; the typed struct is derived from it.
    let composer_json: serde_json::Value = serde_json::from_str(&json_content)
        .map_err(|e| anyhow::anyhow!("invalid composer.json: {e}"))?;
    let parsed: ComposerJson = serde_json::from_value(composer_json.clone())
        .map_err(|e| anyhow::anyhow!("invalid composer.json fields: {e}"))?;

    let version = version_override
        .map(str::to_owned)
        .or(parsed.version)
        .ok_or_else(|| anyhow::anyhow!(
            "composer.json has no 'version' field and no version was provided"
        ))?;

    // Validate name: exactly "vendor/package" with safe characters in each segment.
    // A bare contains('/') check would allow traversal sequences like "a/../../etc".
    let (vendor_seg, rest) = parsed.name.split_once('/').ok_or_else(|| {
        anyhow::anyhow!(
            "composer package name '{}' must be in vendor/package format",
            parsed.name
        )
    })?;
    if rest.contains('/')
        || !is_valid_composer_name_segment(vendor_seg)
        || !is_valid_composer_name_segment(rest)
    {
        anyhow::bail!(
            "composer package name '{}' contains invalid characters or extra path components",
            parsed.name
        );
    }

    Ok(ComposerPackageMeta {
        name: parsed.name,
        version,
        description: parsed.description,
        composer_json,
    })
}

/// Returns true when every character in `s` is alphanumeric, a hyphen, underscore, or dot.
/// Used to validate both Composer package name segments and ZIP path components.
fn is_valid_composer_name_segment(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn find_composer_json(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> anyhow::Result<String> {
    use std::io::Read;

    // Try root-level composer.json first.
    if let Ok(mut f) = archive.by_name("composer.json") {
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        return Ok(s);
    }

    // Fall back to a single top-level directory (GitHub zipball: vendor-pkg-abc123/composer.json).
    // Collect ALL candidates; if more than one top-level directory contains a composer.json the
    // archive is ambiguous and we reject it rather than silently picking a non-deterministic one.
    let candidates: Vec<String> = archive
        .file_names()
        .filter(|n| {
            let mut parts = n.splitn(3, '/');
            parts.next(); // top-level dir
            parts.next() == Some("composer.json") && parts.next().is_none()
        })
        .map(str::to_owned)
        .collect();

    let nested = match candidates.len() {
        0 => anyhow::bail!("composer.json not found in ZIP archive"),
        1 => candidates.into_iter().next().expect("invariant: len == 1"),
        _ => anyhow::bail!(
            "ambiguous ZIP: multiple top-level directories contain composer.json ({})",
            candidates.join(", ")
        ),
    };

    let mut f = archive.by_name(&nested)?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn client(url: &str) -> ComposerRegistryClient {
        ComposerRegistryClient::new(url, &Default::default()).unwrap()
    }

    /// Build a minimal Packagist v2 JSON response for one package + version.
    fn p2_json(package: &str, version: &str, dist_url: &str) -> String {
        serde_json::json!({
            "packages": {
                package: [{
                    "version": version,
                    "dist": {
                        "type": "zip",
                        "url": dist_url,
                        "shasum": "deadbeef00000000000000000000000000000000"
                    },
                    "time": "2024-06-01T12:00:00+00:00",
                    "description": "A test package"
                }]
            },
            "minified": "composer/2.0"
        })
        .to_string()
    }

    // ── resolve_metadata ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn resolve_metadata_returns_correct_fields() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/p2/symfony/console.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(p2_json("symfony/console", "v7.2.0", "https://example.com/dist.zip"))
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "symfony/console", "v7.2.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();

        assert_eq!(meta.id.version, "v7.2.0");
        assert_eq!(
            meta.checksum.as_deref(),
            Some("deadbeef00000000000000000000000000000000")
        );
        assert!(meta.published_at.is_some());
        // No download_url when artifact is None
        assert!(meta.download_url.is_none());
    }

    #[tokio::test]
    async fn resolve_metadata_sets_download_url_for_dist_artifact() {
        let mut server = Server::new_async().await;
        let dist_url = format!("{}/dist/symfony-console.zip", server.url());
        let _mock = server
            .mock("GET", "/p2/symfony/console.json")
            .with_status(200)
            .with_body(p2_json("symfony/console", "v7.2.0", &dist_url))
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "symfony/console", "v7.2.0").with_artifact("dist");
        let meta = c.resolve_metadata(&pkg).await.unwrap();

        assert_eq!(meta.download_url.as_deref(), Some(dist_url.as_str()));
    }

    #[tokio::test]
    async fn resolve_metadata_p2_artifact_returns_url_without_upstream_call() {
        // artifact="p2" must return immediately — no p2 endpoint should be hit.
        let server = Server::new_async().await;
        // No mock registered — any HTTP call would panic.
        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "vendor/package", "_index").with_artifact("p2");
        let meta = c.resolve_metadata(&pkg).await.unwrap();

        assert!(
            meta.download_url
                .as_deref()
                .unwrap_or("")
                .contains("/p2/vendor/package.json"),
            "download_url must point to the p2 endpoint"
        );
    }

    #[tokio::test]
    async fn resolve_metadata_package_not_found_returns_not_found_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/p2/missing/pkg.json")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "missing/pkg", "v1.0.0");
        assert!(matches!(c.resolve_metadata(&pkg).await, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn resolve_metadata_version_not_in_p2_returns_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/p2/vendor/pkg.json")
            .with_status(200)
            .with_body(p2_json("vendor/pkg", "v1.0.0", "https://example.com/a.zip"))
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "vendor/pkg", "v9.9.9");
        assert!(matches!(c.resolve_metadata(&pkg).await, Err(CoreError::NotFound(_))));
    }

    // ── fetch_artifact ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_artifact_p2_streams_raw_json_bytes() {
        let mut server = Server::new_async().await;
        let body = p2_json("vendor/pkg", "v1.0.0", "https://example.com/dist.zip");
        let _mock = server
            .mock("GET", "/p2/vendor/pkg.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(body.clone())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "vendor/pkg", "_index").with_artifact("p2");
        let fetched = c.fetch_artifact(&pkg).await.unwrap();
        let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, body.as_bytes());
    }

    #[tokio::test]
    async fn fetch_artifact_dist_streams_zip_bytes() {
        let mut server = Server::new_async().await;
        let dist_path = "/archives/vendor-pkg-v1.0.0.zip";
        let dist_url = format!("{}{}", server.url(), dist_path);
        let zip_bytes: &[u8] = b"PK\x03\x04fake-zip-content";

        let _p2_mock = server
            .mock("GET", "/p2/vendor/pkg.json")
            .with_status(200)
            .with_body(p2_json("vendor/pkg", "v1.0.0", &dist_url))
            .create_async()
            .await;

        let _zip_mock = server
            .mock("GET", dist_path)
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(zip_bytes)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "vendor/pkg", "v1.0.0").with_artifact("dist");
        let fetched = c.fetch_artifact(&pkg).await.unwrap();
        let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, zip_bytes);
    }

    #[tokio::test]
    async fn fetch_artifact_not_found_returns_not_found_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/p2/missing/pkg.json")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "missing/pkg", "v1.0.0").with_artifact("dist");
        assert!(matches!(c.fetch_artifact(&pkg).await, Err(CoreError::NotFound(_))));
    }

    #[tokio::test]
    async fn fetch_artifact_dist_propagates_cache_control() {
        let mut server = Server::new_async().await;
        let dist_path = "/dist/pkg.zip";
        let dist_url = format!("{}{}", server.url(), dist_path);

        let _p2_mock = server
            .mock("GET", "/p2/vendor/pkg.json")
            .with_status(200)
            .with_body(p2_json("vendor/pkg", "v1.0.0", &dist_url))
            .create_async()
            .await;

        let _zip_mock = server
            .mock("GET", dist_path)
            .with_status(200)
            .with_header("cache-control", "max-age=86400")
            .with_body(b"data".as_slice())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("pkgist", "vendor/pkg", "v1.0.0").with_artifact("dist");
        let fetched = c.fetch_artifact(&pkg).await.unwrap();
        assert_eq!(fetched.cache_control.as_deref(), Some("max-age=86400"));
    }

    // ── list_versions ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_versions_returns_all_versions() {
        let mut server = Server::new_async().await;
        let body = serde_json::json!({
            "packages": {
                "symfony/console": [
                    {"version": "v7.2.0"},
                    {"version": "v7.1.0"},
                    {"version": "v6.4.0"}
                ]
            }
        })
        .to_string();

        let _mock = server
            .mock("GET", "/p2/symfony/console.json")
            .with_status(200)
            .with_body(body)
            .create_async()
            .await;

        let c = client(&server.url());
        let versions = c.list_versions("symfony/console").await.unwrap();
        assert_eq!(versions.len(), 3);
        assert!(versions.contains(&"v7.2.0".to_owned()));
        assert!(versions.contains(&"v6.4.0".to_owned()));
    }

    #[tokio::test]
    async fn list_versions_not_found_returns_error() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/p2/unknown/pkg.json")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        assert!(matches!(
            c.list_versions("unknown/pkg").await,
            Err(CoreError::NotFound(_))
        ));
    }

    // ── parse_composer_zip ────────────────────────────────────────────────────

    fn make_zip_with_file(filename: &str, content: &[u8]) -> Bytes {
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file(filename, opts).unwrap();
            writer.write_all(content).unwrap();
            writer.finish().unwrap();
        }
        Bytes::from(buf.into_inner())
    }

    fn cjson(name: &str, version: &str) -> Vec<u8> {
        serde_json::json!({ "name": name, "version": version, "description": "test" })
            .to_string()
            .into_bytes()
    }

    #[test]
    fn parse_composer_zip_root_level() {
        let data = make_zip_with_file("composer.json", &cjson("vendor/mypkg", "v1.0.0"));
        let meta = parse_composer_zip(&data, None).unwrap();
        assert_eq!(meta.name, "vendor/mypkg");
        assert_eq!(meta.version, "v1.0.0");
    }

    #[test]
    fn parse_composer_zip_github_style_nested() {
        // GitHub zipball layout: <vendor>-<pkg>-<sha>/composer.json
        let data = make_zip_with_file(
            "vendor-mypkg-abc1234/composer.json",
            &cjson("vendor/mypkg", "v2.3.0"),
        );
        let meta = parse_composer_zip(&data, None).unwrap();
        assert_eq!(meta.name, "vendor/mypkg");
        assert_eq!(meta.version, "v2.3.0");
    }

    #[test]
    fn parse_composer_zip_version_override_wins() {
        let data = make_zip_with_file("composer.json", &cjson("vendor/mypkg", "v1.0.0"));
        let meta = parse_composer_zip(&data, Some("v99.0.0")).unwrap();
        assert_eq!(meta.version, "v99.0.0");
    }

    #[test]
    fn parse_composer_zip_no_version_field_without_override_returns_error() {
        let json = serde_json::json!({ "name": "vendor/pkg", "description": "no version" })
            .to_string()
            .into_bytes();
        let data = make_zip_with_file("composer.json", &json);
        assert!(parse_composer_zip(&data, None).is_err());
    }

    #[test]
    fn parse_composer_zip_no_composer_json_returns_error() {
        let data = make_zip_with_file("README.md", b"hello world");
        assert!(parse_composer_zip(&data, None).is_err());
    }

    #[test]
    fn parse_composer_zip_invalid_name_no_slash_returns_error() {
        let json = serde_json::json!({ "name": "noslash", "version": "v1.0.0" })
            .to_string()
            .into_bytes();
        let data = make_zip_with_file("composer.json", &json);
        assert!(parse_composer_zip(&data, None).is_err());
    }

    #[test]
    fn parse_composer_zip_preserves_full_json() {
        let data = make_zip_with_file("composer.json", &cjson("vendor/mypkg", "v1.0.0"));
        let meta = parse_composer_zip(&data, None).unwrap();
        assert_eq!(meta.composer_json["name"], "vendor/mypkg");
        assert_eq!(meta.composer_json["description"], "test");
    }

    #[test]
    fn parse_composer_zip_path_traversal_name_rejected() {
        // Names with '..' components must be rejected to prevent storage path traversal.
        for bad_name in &["vendor/../../etc/shadow", "a/../b/c", "../vendor/pkg", "vendor/pkg/extra"] {
            let json = serde_json::json!({ "name": bad_name, "version": "v1.0.0" })
                .to_string()
                .into_bytes();
            let data = make_zip_with_file("composer.json", &json);
            assert!(
                parse_composer_zip(&data, None).is_err(),
                "expected error for name '{bad_name}'"
            );
        }
    }

    #[test]
    fn parse_composer_zip_multi_root_zip_rejected() {
        // A ZIP with two top-level directories each containing composer.json is ambiguous
        // and must be rejected rather than silently picking one.
        use std::io::Write as _;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            writer.start_file("dir-a/composer.json", opts).unwrap();
            writer.write_all(&cjson("vendor/a", "v1.0.0")).unwrap();
            writer.start_file("dir-b/composer.json", opts).unwrap();
            writer.write_all(&cjson("vendor/b", "v2.0.0")).unwrap();
            writer.finish().unwrap();
        }
        let data = Bytes::from(buf.into_inner());
        assert!(parse_composer_zip(&data, None).is_err(), "ambiguous multi-root ZIP must fail");
    }
}
