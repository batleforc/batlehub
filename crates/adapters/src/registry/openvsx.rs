use async_trait::async_trait;
use chrono::DateTime;
use futures::TryStreamExt;
use serde::Deserialize;
use std::collections::HashMap;

use proxy_cache_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{ArtifactStream, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// OpenVSX registry client (open-vsx.org or compatible).
///
/// Supported `PackageId` conventions:
/// - `name` format: `"{publisher}.{extension}"` (e.g. `"ms-python.python"`)
/// - `version = "latest"` → current latest version
/// - `version = "1.2.3"`  → specific semver version
/// - `artifact = Some("vsix")` → stream the `.vsix` extension package
pub struct OpenVsxRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl OpenVsxRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> Self {
        let builder = reqwest::Client::builder()
            .user_agent("proxy-cache/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)
            .expect("failed to build OpenVSX HTTP client");
        Self { http, base_url: base_url.into(), basic_auth: opts.basic_auth.clone() }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    fn parse_id(name: &str) -> Result<(&str, &str), CoreError> {
        name.split_once('.')
            .ok_or_else(|| CoreError::Registry(format!(
                "invalid OpenVSX extension id '{name}': expected '{{publisher}}.{{name}}'"
            )))
    }
}

// ── Serde types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct OpenVsxExtension {
    namespace: String,
    #[allow(dead_code)]
    name: String,
    version: String,
    timestamp: Option<String>,
    #[serde(default)]
    files: OpenVsxFiles,
    #[serde(rename = "allVersions", default)]
    all_versions: HashMap<String, String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    description: Option<String>,
    #[serde(rename = "downloadCount", default)]
    download_count: u64,
    verified: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenVsxFiles {
    download: Option<String>,
    signature: Option<String>,
    manifest: Option<String>,
    icon: Option<String>,
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for OpenVsxRegistryClient {
    fn registry_type(&self) -> &str {
        "openvsx"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;
        let ext = self.fetch_extension(publisher, ext_name, &pkg.version).await?;

        let published_at = ext.timestamp
            .as_deref()
            .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let download_url = if pkg.artifact.as_deref() == Some("vsix") {
            ext.files.download.clone()
        } else {
            None
        };

        let is_signed = Some(ext.files.signature.is_some());

        let extra = serde_json::json!({
            "resolved_version": ext.version,
            "namespace": ext.namespace,
            "display_name": ext.display_name,
            "description": ext.description,
            "download_count": ext.download_count,
            "verified": ext.verified,
            "manifest_url": ext.files.manifest,
            "icon_url": ext.files.icon,
            "all_versions_count": ext.all_versions.len(),
        });

        Ok(PackageMetadata {
            id: PackageId {
                version: ext.version,
                ..pkg.clone()
            },
            published_at,
            download_url,
            checksum: None,
            is_signed,
            extra,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<ArtifactStream, CoreError> {
        let (publisher, ext_name) = Self::parse_id(&pkg.name)?;
        let ext = self.fetch_extension(publisher, ext_name, &pkg.version).await?;

        let download_url = ext.files.download.ok_or_else(|| {
            CoreError::NotFound(format!(
                "no VSIX download available for {}.{} v{}",
                publisher, ext_name, pkg.version
            ))
        })?;

        tracing::debug!(url = %download_url, "fetching OpenVSX VSIX");

        let response = self
            .get(&download_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        let stream = response
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(Box::pin(stream))
    }
}

impl OpenVsxRegistryClient {
    async fn fetch_extension(
        &self,
        publisher: &str,
        name: &str,
        version: &str,
    ) -> Result<OpenVsxExtension, CoreError> {
        let url = if version == "latest" {
            format!("{}/api/{}/{}", self.base_url, publisher, name)
        } else {
            format!("{}/api/{}/{}/{}", self.base_url, publisher, name, version)
        };

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "OpenVSX extension {publisher}.{name}@{version} not found"
            )));
        }

        resp.error_for_status()
            .map_err(|e| CoreError::Registry(e.to_string()))?
            .json::<OpenVsxExtension>()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))
    }
}
