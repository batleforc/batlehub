use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// crates.io (or compatible) registry client.
///
/// Supported `PackageId` conventions:
/// - `version = "latest"`   → resolve via crates.io API (max_version)
/// - `version = "1.2.3"`    → specific version metadata
/// - `artifact = Some("dl")` → stream the `.crate` file for that version
pub struct CargoRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl CargoRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
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
struct CratesIoResponse {
    #[serde(rename = "crate")]
    krate: CrateInfo,
    versions: Vec<CrateVersion>,
}

#[derive(Debug, Deserialize)]
struct CrateInfo {
    max_version: String,
}

#[derive(Debug, Deserialize)]
struct CrateVersion {
    num: String,
    #[serde(rename = "dl_path")]
    dl_path: String,
    checksum: Option<String>,
    created_at: Option<String>,
    yanked: bool,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for CargoRegistryClient {
    fn registry_type(&self) -> &str {
        "cargo"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let resp = self.fetch_crate_info(&pkg.name).await?;
        let version = Self::resolve_version(&resp, pkg)?;
        let resolved_version = version.num.clone();

        let download_url = if pkg.artifact.as_deref() == Some("dl") {
            // dl_path is a relative path like /api/v1/crates/serde/1.0.0/download
            Some(format!("{}{}", self.base_url, version.dl_path))
        } else {
            None
        };

        let published_at = version
            .created_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let extra = serde_json::json!({
            "resolved_version": resolved_version,
            "dl_path": version.dl_path,
            "yanked": version.yanked,
        });

        Ok(PackageMetadata {
            id: PackageId {
                version: resolved_version,
                ..pkg.clone()
            },
            published_at,
            download_url,
            checksum: version.checksum.clone(),
            is_signed: None,
            extra,
            cache_control: None,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let resp = self.fetch_crate_info(package).await?;
        // Return non-yanked versions sorted oldest-first (crates.io returns newest-first).
        let mut versions: Vec<String> = resp.versions.into_iter()
            .filter(|v| !v.yanked)
            .map(|v| v.num)
            .collect();
        versions.reverse();
        Ok(versions)
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let resp = self.fetch_crate_info(&pkg.name).await?;
        let version = Self::resolve_version(&resp, pkg)?;
        let url = format!("{}{}", self.base_url, version.dl_path);
        tracing::debug!(url = %url, "fetching cargo crate");

        let response = self
            .get(&url)
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

impl CargoRegistryClient {
    fn resolve_version<'a>(
        resp: &'a CratesIoResponse,
        pkg: &PackageId,
    ) -> Result<&'a CrateVersion, CoreError> {
        let resolved = if pkg.version == "latest" {
            resp.krate.max_version.as_str()
        } else {
            pkg.version.as_str()
        };
        resp.versions
            .iter()
            .find(|v| v.num == resolved && !v.yanked)
            .ok_or_else(|| CoreError::NotFound(format!(
                "crate {}@{} not found or yanked",
                pkg.name, resolved
            )))
    }

    async fn fetch_crate_info(&self, name: &str) -> Result<CratesIoResponse, CoreError> {
        let url = format!("{}/api/v1/crates/{}", self.base_url, name);
        let resp = self.get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("crate {name} not found")));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<CratesIoResponse>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
    }
}
