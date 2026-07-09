use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
    /// Return the `/api/v1/gems/{name}.json`-compatible info for the latest gem version.
    pub async fn get_rubygems_gem_info(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        let latest = Self::latest_stable_or_newest(&versions).ok_or_else(|| {
            CoreError::NotFound(format!(
                "gem '{name}' not found in local registry '{registry}'"
            ))
        })?;
        let meta = &latest.index_metadata;
        Ok(serde_json::json!({
            "name": name,
            "version": latest.version,
            "platform": meta.get("platform").and_then(|v| v.as_str()).unwrap_or("ruby"),
            "summary": meta.get("summary"),
            "authors": meta.get("authors").and_then(|a| a.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
                .unwrap_or_default(),
            "sha": meta.get("sha"),
            "created_at": latest.published_at.to_rfc3339(),
            "yanked": latest.yanked,
        }))
    }

    /// Return the `/api/v1/versions/{name}.json`-compatible array for all gem versions.
    pub async fn get_rubygems_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<serde_json::Value>, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, name, identity, "gem")
            .await?;
        let result = versions
            .into_iter()
            .rev() // newest-first to match rubygems.org API
            .map(|pkg| {
                let meta = &pkg.index_metadata;
                serde_json::json!({
                    "number": pkg.version,
                    "platform": meta.get("platform").and_then(|v| v.as_str()).unwrap_or("ruby"),
                    "authors": meta.get("authors").and_then(|a| a.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "))
                        .unwrap_or_default(),
                    "summary": meta.get("summary"),
                    "sha": meta.get("sha"),
                    "created_at": pkg.published_at.to_rfc3339(),
                    "prerelease": Self::is_prerelease(&pkg.version),
                    "yanked": pkg.yanked,
                })
            })
            .collect();
        Ok(result)
    }
}
