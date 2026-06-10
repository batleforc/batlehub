use super::{CoreError, Identity, LocalRegistryService, TerraformPlatform};

impl LocalRegistryService {
    /// Build the Terraform module versions envelope from `local_packages`.
    pub async fn get_tf_module_versions_response(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, name, identity, "module")
            .await?;
        let version_list: Vec<serde_json::Value> = versions
            .iter()
            .filter(|v| !v.yanked)
            .map(|v| serde_json::json!({"version": v.version}))
            .collect();
        Ok(serde_json::json!({"modules": [{"versions": version_list}]}))
    }

    /// Build the Terraform provider versions envelope from `local_packages`.
    pub async fn get_tf_provider_versions_response(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, name, identity, "provider")
            .await?;
        let version_list: Vec<serde_json::Value> = versions
            .iter()
            .filter(|v| !v.yanked)
            .map(|v| {
                let meta = &v.index_metadata;
                let protocols = meta
                    .get("protocols")
                    .and_then(|p| p.as_array())
                    .cloned()
                    .unwrap_or_default();
                let platforms = meta
                    .get("platforms")
                    .and_then(|p| p.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|p| {
                                serde_json::json!({
                                    "os": p.get("os").and_then(|v| v.as_str()).unwrap_or(""),
                                    "arch": p.get("arch").and_then(|v| v.as_str()).unwrap_or(""),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                serde_json::json!({
                    "version": v.version,
                    "protocols": protocols,
                    "platforms": platforms,
                })
            })
            .collect();
        Ok(serde_json::json!({"versions": version_list}))
    }

    /// Build the Terraform provider download-info response for a specific version+platform.
    /// Rewrites `download_url` to point at `base_url`.
    pub async fn get_tf_provider_download_response(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        platform: TerraformPlatform<'_>,
        base_url: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let TerraformPlatform { os, arch } = platform;
        self.check_visibility(registry, name, identity).await?;
        self.check_prerelease_access(registry, version, identity)
            .await?;
        let versions = self.backend.get_versions(registry, name).await?;
        let pkg = versions
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "provider {name}@{version} not found in registry '{registry}'"
                ))
            })?;

        let platforms = pkg
            .index_metadata
            .get("platforms")
            .and_then(|p| p.as_array())
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "provider {name}@{version} has no platforms metadata"
                ))
            })?;

        let platform = platforms
            .iter()
            .find(|p| {
                p.get("os").and_then(|v| v.as_str()) == Some(os)
                    && p.get("arch").and_then(|v| v.as_str()) == Some(arch)
            })
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "provider {name}@{version} has no platform {os}/{arch}"
                ))
            })?;

        // Extract namespace/type from name like "providers/{ns}/{type}"
        let parts: Vec<&str> = name.splitn(3, '/').collect();
        let (ns, ptype) = if parts.len() == 3 {
            (parts[1], parts[2])
        } else {
            ("", "")
        };

        let base = base_url.trim_end_matches('/');
        let download_url = format!(
            "{base}/proxy/{registry}/v1/providers/{ns}/{ptype}/{version}/artifact/{os}/{arch}"
        );

        let mut resp = platform.clone();
        if let Some(obj) = resp.as_object_mut() {
            obj.insert("os".to_owned(), serde_json::json!(os));
            obj.insert("arch".to_owned(), serde_json::json!(arch));
            obj.insert("download_url".to_owned(), serde_json::json!(download_url));
            let meta = &pkg.index_metadata;
            obj.entry("protocols").or_insert_with(|| {
                meta.get("protocols")
                    .cloned()
                    .unwrap_or(serde_json::json!([]))
            });
            obj.entry("signing_keys")
                .or_insert_with(|| serde_json::json!({"gpg_public_keys": []}));
        }
        Ok(resp)
    }
}
