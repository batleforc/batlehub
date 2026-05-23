use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

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
        Ok(Self { http, base_url: base_url.into(), basic_auth: opts.basic_auth.clone() })
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
        let resolved_version = packument
            .dist_tags
            .get(&pkg.version)
            .cloned()
            .unwrap_or_else(|| pkg.version.clone());

        let version_meta = packument
            .versions
            .get(&resolved_version)
            .ok_or_else(|| CoreError::NotFound(format!(
                "npm package {}@{} not found",
                pkg.name, resolved_version
            )))?;

        let download_url = if pkg.artifact.as_deref() == Some("tarball") {
            Some(version_meta.dist.tarball.clone())
        } else {
            None
        };

        let checksum = if !version_meta.dist.integrity.is_empty() {
            Some(version_meta.dist.integrity.clone())
        } else if !version_meta.dist.shasum.is_empty() {
            Some(version_meta.dist.shasum.clone())
        } else {
            None
        };

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

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        // Resolve the tarball URL for this version.
        let packument = self.fetch_packument(&pkg.name).await?;
        let resolved_version = packument
            .dist_tags
            .get(&pkg.version)
            .cloned()
            .unwrap_or_else(|| pkg.version.clone());

        let version_meta = packument
            .versions
            .get(&resolved_version)
            .ok_or_else(|| CoreError::NotFound(format!(
                "npm package {}@{} not found",
                pkg.name, resolved_version
            )))?;

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

        Ok(FetchedArtifact { stream: Box::pin(stream), cache_control })
    }
}

impl NpmRegistryClient {
    async fn fetch_packument(&self, name: &str) -> Result<NpmPackument, CoreError> {
        // Scoped packages: @scope/pkg → must be percent-encoded as @scope%2Fpkg
        let encoded = name.replace('/', "%2F");
        let url = format!("{}/{}", self.base_url, encoded);

        let resp = self.get(&url)
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
