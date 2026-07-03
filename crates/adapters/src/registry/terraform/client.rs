use async_trait::async_trait;
use futures::TryStreamExt;

use super::{modules, providers, TerraformRegistryClient};
use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::super::http_client::{cache_control, to_registry_error};

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for TerraformRegistryClient {
    fn registry_type(&self) -> &str {
        "terraform"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let url = self.artifact_url(pkg)?;

        let resp =
            self.get(&url).send().await.map_err(|e| {
                CoreError::Registry(format!("terraform metadata request failed: {e}"))
            })?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "terraform resource not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(CoreError::Registry(format!(
                "terraform metadata request returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = cache_control(&resp);

        // Fetch per-version publish timestamp for specific-version requests.
        // Version listings ("versions") have no meaningful single timestamp.
        let published_at = if pkg.version != "versions" {
            modules::fetch_version_published_at(self, pkg).await
        } else {
            None
        };

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
            download_url: Some(url),
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let url = self.artifact_url(pkg)?;

        tracing::debug!(url = %url, "fetching Terraform artifact");

        let response = self.get(&url).send().await.map_err(to_registry_error)?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "terraform artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !response.status().is_success() && response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(CoreError::Registry(format!(
                "terraform upstream returned {} for {}",
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
        let url = format!("{base}/v1/{package}/versions");

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

        modules::parse_versions(package, &body)
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        let Some(ref base) = self.search_base else {
            return Ok(vec![]);
        };

        let per = limit.min(25);

        // 1. Full-text module search (registry protocol v1 — always works).
        // 2. Provider lookup strategy — the Terraform Registry Protocol has no
        //    full-text provider search. Uses two heuristics (namespace + exact).
        let mut results: Vec<UpstreamPackage> = Vec::new();

        results.extend(providers::search_modules(self, base, query, per).await);
        results.extend(providers::search_providers(self, base, query, per).await);

        // Deduplicate by name
        let mut seen = std::collections::HashSet::new();
        results.retain(|r| seen.insert(r.name.clone()));

        Ok(results)
    }
}
