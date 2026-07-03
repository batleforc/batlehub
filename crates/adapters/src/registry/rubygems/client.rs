use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use super::super::http_client::{cache_control, percent_encode, to_registry_error};
use super::models::{GemInfo, GemVersion};
use super::{models, CoreError, RubyGemsRegistryClient};
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};
use models::GemMetadata;

// ── Local publish helpers ─────────────────────────────────────────────────────

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

/// Extract the gem version from the nested Gem::Version YAML block:
///   version: !ruby/object:Gem::Version
///     version: '1.0.0'
fn extract_gem_version(yaml: &str) -> Option<String> {
    let mut after_version_key = false;
    for line in yaml.lines() {
        if line.starts_with("version:") {
            after_version_key = true;
            continue;
        }
        if !after_version_key {
            continue;
        }
        let trimmed = line.trim_start_matches(' ');
        if let Some(rest) = trimmed.strip_prefix("version: ") {
            let v = rest.trim();
            if !v.starts_with('!') {
                return Some(strip_yaml_quotes(v).to_owned());
            }
        }
        if !line.starts_with(' ') {
            after_version_key = false;
        }
    }
    None
}

pub(super) fn parse_gem_yaml(yaml: &str) -> Result<GemMetadata, CoreError> {
    let name = extract_yaml_value(yaml, "name: ")
        .ok_or_else(|| CoreError::Registry("rubygems: gem name not found in metadata".to_owned()))?
        .to_owned();

    let version = extract_gem_version(yaml).ok_or_else(|| {
        CoreError::Registry("rubygems: gem version not found in metadata".to_owned())
    })?;

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
            let body = resp.bytes().await.map_err(to_registry_error)?;
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

        let response = self.get(&url).send().await.map_err(to_registry_error)?;

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

        let stream = response.bytes_stream().map_err(to_registry_error);

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/api/v1/versions/{package}.json");

        let resp = self.get(&url).send().await.map_err(to_registry_error)?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }

        let body = resp
            .error_for_status()
            .map_err(to_registry_error)?
            .bytes()
            .await
            .map_err(to_registry_error)?;

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
        let res = self.get(&url).send().await.map_err(to_registry_error)?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let gems: Vec<GemSearchResult> = res.json().await.map_err(to_registry_error)?;

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
