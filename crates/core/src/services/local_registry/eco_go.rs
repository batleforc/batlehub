use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
    /// Return newline-delimited version list for a locally published Go module.
    /// Returns `CoreError::NotFound` if the module has never been published here.
    pub async fn get_go_version_list(
        &self,
        registry: &str,
        module: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        let versions = self
            .load_visible_versions(registry, module, identity)
            .await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "module '{}' not found in local registry '{}'",
                module, registry
            )));
        }
        let list = versions
            .iter()
            .filter_map(|v| {
                v.index_metadata
                    .get("Version")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(list)
    }

    /// Return the `.info` JSON for a specific Go module version.
    pub async fn get_go_info(
        &self,
        registry: &str,
        module: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        self.check_visibility(registry, module, identity).await?;
        self.check_prerelease_access(registry, version, identity)
            .await?;
        let pkg = self
            .backend
            .get_versions(registry, module)
            .await?
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    module, version, registry
                ))
            })?;
        let v = pkg
            .index_metadata
            .get("Version")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(version));
        let t = pkg
            .index_metadata
            .get("Time")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.published_at.to_rfc3339()));
        Ok(serde_json::json!({ "Version": v, "Time": t }))
    }

    /// Return the `go.mod` content for a specific Go module version.
    pub async fn get_go_mod(
        &self,
        registry: &str,
        module: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        self.check_visibility(registry, module, identity).await?;
        self.check_prerelease_access(registry, version, identity)
            .await?;
        let pkg = self
            .backend
            .get_versions(registry, module)
            .await?
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    module, version, registry
                ))
            })?;
        pkg.index_metadata
            .get("go_mod")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "go.mod not found for {}@{} in registry '{}'",
                    module, version, registry
                ))
            })
    }

    /// Return the `.info` JSON for the most recently published Go module version.
    pub async fn get_go_latest(
        &self,
        registry: &str,
        module: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self
            .load_visible_versions(registry, module, identity)
            .await?;
        let pkg = versions
            .iter()
            .rev()
            .find(|v| !Self::is_prerelease(&v.version))
            .or_else(|| versions.last())
            .cloned()
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "module '{}' not found in local registry '{}'",
                    module, registry
                ))
            })?;
        let v = pkg
            .index_metadata
            .get("Version")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.version));
        let t = pkg
            .index_metadata
            .get("Time")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.published_at.to_rfc3339()));
        Ok(serde_json::json!({ "Version": v, "Time": t }))
    }
}
