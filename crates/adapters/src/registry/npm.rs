use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, percent_encode, UpstreamHttpOptions};

/// npm registry client (registry.npmjs.org or compatible).
///
/// Supported `PackageId` conventions:
/// - `version = "latest"` or a dist-tag  → resolve via packument, return metadata
/// - `version = "1.2.3"` (exact semver)  → version-specific metadata
/// - `artifact = Some("tarball")`        → stream the `.tgz` for that version
pub struct NpmRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl NpmRegistryClient {
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

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct NpmPackument {
    #[serde(rename = "dist-tags")]
    dist_tags: HashMap<String, String>,
    versions: HashMap<String, NpmVersionMeta>,
    /// Per-version publish timestamps from the full packument.
    /// Keys are version strings (e.g. `"1.2.3"`) plus the special keys
    /// `"created"` and `"modified"`. Only present in the full packument
    /// (`application/json`); absent in the abbreviated install manifest.
    #[serde(default)]
    time: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct NpmVersionMeta {
    #[allow(dead_code)]
    version: String,
    dist: NpmDist,
    #[serde(rename = "_npmUser")]
    npm_user: Option<NpmUser>,
}

#[derive(Debug, Deserialize)]
struct NpmDist {
    tarball: String,
    #[serde(default)]
    integrity: String,
    #[serde(default)]
    shasum: String,
}

#[derive(Debug, Deserialize)]
struct NpmUser {
    name: Option<String>,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for NpmRegistryClient {
    fn registry_type(&self) -> &str {
        "npm"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let packument = self.fetch_packument(&pkg.name).await?;

        // Resolve dist-tag (e.g. "latest") → concrete version string.
        let resolved_version = resolve_dist_tag(&packument.dist_tags, &pkg.version).to_owned();

        let version_meta = packument.versions.get(&resolved_version).ok_or_else(|| {
            CoreError::NotFound(format!(
                "npm package {}@{} not found",
                pkg.name, resolved_version
            ))
        })?;

        let download_url = if pkg.artifact.as_deref() == Some("tarball") {
            Some(version_meta.dist.tarball.clone())
        } else {
            None
        };

        let checksum = pick_checksum(&version_meta.dist);

        let extra = serde_json::json!({
            "resolved_version": resolved_version,
            "tarball": version_meta.dist.tarball,
            "publisher": version_meta.npm_user.as_ref().and_then(|u| u.name.as_deref()),
        });

        let published_at = packument
            .time
            .get(&resolved_version)
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        Ok(PackageMetadata {
            id: PackageId {
                version: resolved_version,
                ..pkg.clone()
            },
            published_at,
            download_url,
            checksum,
            is_signed: None,
            extra,
            cache_control: None,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let packument = self.fetch_packument(package).await?;
        let mut versions: Vec<String> = packument.versions.into_keys().collect();
        // Sort by semver lexicographically as a best-effort ordering (oldest first).
        versions.sort();
        Ok(versions)
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        // Resolve the tarball URL for this version.
        let packument = self.fetch_packument(&pkg.name).await?;
        let resolved_version = resolve_dist_tag(&packument.dist_tags, &pkg.version).to_owned();

        let version_meta = packument.versions.get(&resolved_version).ok_or_else(|| {
            CoreError::NotFound(format!(
                "npm package {}@{} not found",
                pkg.name, resolved_version
            ))
        })?;

        let tarball_url = &version_meta.dist.tarball;
        tracing::debug!(url = %tarball_url, "fetching npm tarball");

        let response = self
            .get(tarball_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let cache_control = response
            .headers()
            .get("cache-control")
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

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(Deserialize)]
        struct SearchResponse {
            objects: Vec<SearchObject>,
        }
        #[derive(Deserialize)]
        struct SearchObject {
            package: SearchPackage,
        }
        #[derive(Deserialize)]
        struct SearchPackage {
            name: String,
            version: String,
            description: Option<String>,
        }

        let url = format!(
            "{}/-/v1/search?text={}&size={}",
            self.base_url,
            percent_encode(query),
            limit.min(50),
        );
        let res = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = res
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(body
            .objects
            .into_iter()
            .map(|o| UpstreamPackage {
                name: o.package.name,
                latest_version: o.package.version,
                description: o.package.description,
            })
            .collect())
    }
}

fn encode_npm_name(name: &str) -> String {
    name.replace('/', "%2F")
}

/// Resolve a dist-tag (e.g. `"latest"`) to a concrete version string.
/// Returns the version unchanged when the tag is not in `dist_tags`.
fn resolve_dist_tag<'a>(dist_tags: &'a HashMap<String, String>, version: &'a str) -> &'a str {
    dist_tags
        .get(version)
        .map(String::as_str)
        .unwrap_or(version)
}

/// Select the best checksum available: `integrity` (preferred) over `shasum`.
fn pick_checksum(dist: &NpmDist) -> Option<String> {
    if !dist.integrity.is_empty() {
        Some(dist.integrity.clone())
    } else if !dist.shasum.is_empty() {
        Some(dist.shasum.clone())
    } else {
        None
    }
}

impl NpmRegistryClient {
    async fn fetch_packument(&self, name: &str) -> Result<NpmPackument, CoreError> {
        // Scoped packages: @scope/pkg → must be percent-encoded as @scope%2Fpkg
        let encoded = encode_npm_name(name);
        let url = format!("{}/{}", self.base_url, encoded);

        let resp = self
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("npm package {name} not found")));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<NpmPackument>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_scoped_package() {
        assert_eq!(encode_npm_name("@scope/pkg"), "@scope%2Fpkg");
        assert_eq!(encode_npm_name("lodash"), "lodash");
    }

    #[test]
    fn resolve_dist_tag_known() {
        let mut tags = HashMap::new();
        tags.insert("latest".to_string(), "1.5.0".to_string());
        assert_eq!(resolve_dist_tag(&tags, "latest"), "1.5.0");
    }

    #[test]
    fn resolve_dist_tag_unknown_passes_through() {
        let tags = HashMap::new();
        assert_eq!(resolve_dist_tag(&tags, "2.0.0"), "2.0.0");
    }

    #[test]
    fn pick_checksum_prefers_integrity() {
        let dist = NpmDist {
            tarball: "https://example.com/pkg.tgz".into(),
            integrity: "sha512-abc".into(),
            shasum: "oldsha".into(),
        };
        assert_eq!(pick_checksum(&dist).as_deref(), Some("sha512-abc"));
    }

    #[test]
    fn pick_checksum_falls_back_to_shasum() {
        let dist = NpmDist {
            tarball: "https://example.com/pkg.tgz".into(),
            integrity: String::new(),
            shasum: "abc123".into(),
        };
        assert_eq!(pick_checksum(&dist).as_deref(), Some("abc123"));
    }

    #[test]
    fn pick_checksum_none_when_both_empty() {
        let dist = NpmDist {
            tarball: "https://example.com/pkg.tgz".into(),
            integrity: String::new(),
            shasum: String::new(),
        };
        assert!(pick_checksum(&dist).is_none());
    }
}
