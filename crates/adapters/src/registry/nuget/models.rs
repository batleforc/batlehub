use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::super::http_client::{cache_control, percent_encode};
use super::client::NugetRegistryClient;

// ── Pure helper functions ─────────────────────────────────────────────────────

/// Normalise a NuGet package ID to lower-case (IDs are case-insensitive in the protocol).
pub fn normalize_id(id: &str) -> String {
    id.to_lowercase()
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for NugetRegistryClient {
    fn registry_type(&self) -> &str {
        "nuget"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let id = &pkg.name; // already lowercased by the handler

        match pkg.version.as_str() {
            "__index__" => {
                let url = format!("{}/{}/index.json", self.flat_url, id);
                let cache_control = self
                    .fetch_cache_control(&url, &format!("flat index for '{id}'"))
                    .await?;
                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }

            "__registration__" => {
                let url = format!("{}/{}/index.json", self.reg_url, id);
                let cache_control = self
                    .fetch_cache_control(&url, &format!("registration for '{id}'"))
                    .await?;
                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }

            version => {
                // Specific version: confirm it exists in the flat container, get timestamp.
                let (versions, cache_control) = self.fetch_flat_index(id).await?;
                if !versions.iter().any(|v| v == version) {
                    return Err(CoreError::NotFound(format!(
                        "NuGet package '{id}' version '{version}' not found"
                    )));
                }

                let published_at = self.head_nupkg_last_modified(id, version).await;

                let download_url = Some(format!(
                    "{}/{}/{}/{}.{}.nupkg",
                    self.flat_url, id, version, id, version
                ));

                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at,
                    download_url,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }
        }
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let id = &pkg.name;
        let url = match (pkg.version.as_str(), pkg.artifact.as_deref()) {
            ("__index__", _) => format!("{}/{}/index.json", self.flat_url, id),
            ("__registration__", _) => format!("{}/{}/index.json", self.reg_url, id),
            (version, Some(filename)) => {
                format!("{}/{}/{}/{}", self.flat_url, id, version, filename)
            }
            (version, None) => {
                format!(
                    "{}/{}/{}/{}.{}.nupkg",
                    self.flat_url, id, version, id, version
                )
            }
        };

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("NuGet artifact request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "NuGet artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "NuGet artifact returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

        let stream = resp
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let id = normalize_id(package);
        let (mut versions, _) = self.fetch_flat_index(&id).await?;
        versions.sort();
        Ok(versions)
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(Deserialize)]
        struct SearchResponse {
            data: Vec<SearchEntry>,
        }
        #[derive(Deserialize)]
        struct SearchEntry {
            id: String,
            version: String,
            description: Option<String>,
        }

        let Some(ref search_base) = self.search_base else {
            return Ok(vec![]);
        };

        let url = format!(
            "{}?q={}&take={}&prerelease=false",
            search_base,
            percent_encode(query),
            limit.min(100),
        );

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = resp
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(body
            .data
            .into_iter()
            .map(|e| UpstreamPackage {
                name: e.id,
                latest_version: e.version,
                description: e.description,
            })
            .collect())
    }
}
