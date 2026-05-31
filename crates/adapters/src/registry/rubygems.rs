use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, percent_encode, UpstreamHttpOptions};

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
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
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

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GemInfo {
    #[serde(default)]
    version: String,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GemVersion {
    number: String,
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

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

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
        struct GemInfo {
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

        let gems: Vec<GemInfo> = res
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

// ── Local publish helpers ─────────────────────────────────────────────────────

/// Metadata extracted from a `.gem` archive.
#[derive(Debug, Clone)]
pub struct GemMetadata {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub summary: Option<String>,
    pub authors: Vec<String>,
}

/// Parse a `.gem` file (TAR archive containing `metadata.gz`) and extract gem metadata.
///
/// The `.gem` format is a TAR archive with:
/// - `metadata.gz` — gzip-compressed YAML gem specification
/// - `data.tar.gz` — the gem's actual files
#[cfg(feature = "local-registry")]
pub fn parse_gem_bytes(data: &[u8]) -> Result<GemMetadata, CoreError> {
    use std::io::{Cursor, Read};

    let cursor = Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);

    let mut metadata_bytes: Option<Vec<u8>> = None;
    for entry in archive
        .entries()
        .map_err(|e| CoreError::Registry(format!("rubygems: read gem tar: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| CoreError::Registry(format!("rubygems: gem entry: {e}")))?;
        let is_metadata = entry
            .path()
            .map(|p| p.as_os_str() == "metadata.gz")
            .unwrap_or(false);
        if is_metadata {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| CoreError::Registry(format!("rubygems: read metadata.gz: {e}")))?;
            metadata_bytes = Some(buf);
            break;
        }
    }

    let compressed = metadata_bytes.ok_or_else(|| {
        CoreError::Registry("rubygems: metadata.gz not found in .gem archive".to_owned())
    })?;

    let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
    let mut yaml = String::new();
    decoder
        .read_to_string(&mut yaml)
        .map_err(|e| CoreError::Registry(format!("rubygems: decompress metadata.gz: {e}")))?;

    parse_gem_yaml(&yaml)
}

fn extract_yaml_value<'a>(yaml: &'a str, key: &str) -> Option<&'a str> {
    for line in yaml.lines() {
        let trimmed = line.trim_start_matches(' ');
        if let Some(rest) = trimmed.strip_prefix(key) {
            let v = rest.trim();
            if v.starts_with('!') {
                return None;
            }
            return Some(strip_yaml_quotes(v));
        }
    }
    None
}

fn strip_yaml_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn parse_gem_yaml(yaml: &str) -> Result<GemMetadata, CoreError> {
    let name = extract_yaml_value(yaml, "name: ")
        .ok_or_else(|| CoreError::Registry("rubygems: gem name not found in metadata".to_owned()))?
        .to_owned();

    // Version is inside a nested Gem::Version object:
    //   version: !ruby/object:Gem::Version
    //     version: '1.0.0'
    // We look for a line with leading spaces that contains "version: " followed by a non-'!' value.
    let version = {
        let mut found: Option<String> = None;
        let mut after_version_key = false;
        for line in yaml.lines() {
            if line.starts_with("version:") {
                after_version_key = true;
                continue;
            }
            if after_version_key {
                let trimmed = line.trim_start_matches(' ');
                if let Some(rest) = trimmed.strip_prefix("version: ") {
                    let v = rest.trim();
                    if !v.starts_with('!') {
                        found = Some(strip_yaml_quotes(v).to_owned());
                        break;
                    }
                }
                // Stop looking once we've passed the indented block
                if !line.starts_with(' ') {
                    after_version_key = false;
                }
            }
        }
        found.ok_or_else(|| {
            CoreError::Registry("rubygems: gem version not found in metadata".to_owned())
        })?
    };

    let platform = extract_yaml_value(yaml, "platform: ")
        .unwrap_or("ruby")
        .to_owned();
    let summary = extract_yaml_value(yaml, "summary: ").map(str::to_owned);

    let mut authors = Vec::new();
    let mut in_authors = false;
    for line in yaml.lines() {
        if line == "authors:" || line.starts_with("authors:") {
            in_authors = true;
            continue;
        }
        if in_authors {
            let trimmed = line.trim_start_matches(' ');
            if let Some(author) = trimmed.strip_prefix("- ") {
                authors.push(strip_yaml_quotes(author.trim()).to_owned());
            } else {
                in_authors = false;
            }
        }
    }

    Ok(GemMetadata {
        name,
        version,
        platform,
        summary,
        authors,
    })
}

/// Split a gem filename stem (without `.gem`) into `(name, version)`.
///
/// Gem filenames follow `{name}-{version}` where version starts with a digit.
/// For multi-hyphen gem names like `json-jwt`, the split point is at the first
/// `-` that is immediately followed by a digit.
pub fn split_gem_stem(stem: &str) -> Option<(&str, &str)> {
    let bytes = stem.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            return Some((&stem[..i], &stem[i + 1..]));
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use mockito::Server;

    fn client(url: &str) -> RubyGemsRegistryClient {
        RubyGemsRegistryClient::new(url, &Default::default()).unwrap()
    }

    #[test]
    fn split_gem_stem_simple() {
        assert_eq!(split_gem_stem("rails-7.1.0"), Some(("rails", "7.1.0")));
    }

    #[test]
    fn split_gem_stem_hyphenated_name() {
        assert_eq!(
            split_gem_stem("json-jwt-1.0.0"),
            Some(("json-jwt", "1.0.0"))
        );
    }

    #[test]
    fn split_gem_stem_platform() {
        assert_eq!(
            split_gem_stem("nokogiri-1.10.0-x86_64-linux"),
            Some(("nokogiri", "1.10.0-x86_64-linux"))
        );
    }

    #[test]
    fn split_gem_stem_no_version() {
        assert_eq!(split_gem_stem("rails"), None);
    }

    #[tokio::test]
    async fn list_versions_ok() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/v1/versions/rails.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"[{"number":"7.1.0"},{"number":"7.0.8"},{"number":"6.1.7"}]"#)
            .create_async()
            .await;

        let c = client(&server.url());
        let versions = c.list_versions("rails").await.unwrap();
        // Should be reversed to oldest-first
        assert_eq!(versions, vec!["6.1.7", "7.0.8", "7.1.0"]);
    }

    #[tokio::test]
    async fn list_versions_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/v1/versions/unknown-gem.json")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let versions = c.list_versions("unknown-gem").await.unwrap();
        assert!(versions.is_empty());
    }

    #[tokio::test]
    async fn fetch_artifact_gem_download() {
        let body = b"fake-gem-bytes";
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/gems/rails-7.1.0.gem")
            .with_status(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(body.as_slice())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("rg", "rails", "7.1.0").with_artifact("gem");
        let fetched = c.fetch_artifact(&pkg).await.unwrap();
        let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, body);
    }

    #[tokio::test]
    async fn fetch_artifact_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/gems/missing-1.0.0.gem")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("rg", "missing", "1.0.0").with_artifact("gem");
        assert!(matches!(
            c.fetch_artifact(&pkg).await,
            Err(CoreError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn fetch_artifact_index() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/specs.4.8.gz")
            .with_status(200)
            .with_body(b"gz-data".as_slice())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("rg", "_index", "specs");
        let fetched = c.fetch_artifact(&pkg).await.unwrap();
        let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
        let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
        assert_eq!(content, b"gz-data");
    }

    #[tokio::test]
    async fn resolve_metadata_parses_gem_info() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/api/v1/gems/rails.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("cache-control", "max-age=60")
            .with_body(
                r#"{"name":"rails","version":"7.1.0","created_at":"2023-10-04T12:00:00.000Z","sha":"abc123"}"#,
            )
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("rg", "rails", "info");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.checksum.as_deref(), Some("abc123"));
        assert_eq!(meta.cache_control.as_deref(), Some("max-age=60"));
        assert!(meta.published_at.is_some());
    }

    #[cfg(feature = "local-registry")]
    #[test]
    fn parse_gem_yaml_basic() {
        let yaml = r#"--- !ruby/object:Gem::Specification
name: rails
version: !ruby/object:Gem::Version
  version: '7.1.0'
platform: ruby
authors:
- David Heinemeier Hansson
summary: Full-stack web application framework.
"#;
        let meta = parse_gem_yaml(yaml).unwrap();
        assert_eq!(meta.name, "rails");
        assert_eq!(meta.version, "7.1.0");
        assert_eq!(meta.platform, "ruby");
        assert_eq!(
            meta.summary.as_deref(),
            Some("Full-stack web application framework.")
        );
        assert_eq!(meta.authors, vec!["David Heinemeier Hansson"]);
    }
}
