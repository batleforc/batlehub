use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
    /// Build a Packagist v2-compatible p2 JSON response for a locally published package.
    ///
    /// Returns `CoreError::NotFound` when no versions are published for `name`.
    pub async fn get_composer_p2_response(
        &self,
        registry: &str,
        name: &str,
        base_url: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;

        // Exclude yanked versions: Composer clients have no standard way to
        // interpret a `yanked` field, so they would happily install yanked releases.
        let versions: Vec<_> = versions.into_iter().filter(|p| !p.yanked).collect();

        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "composer package '{name}' not found in local registry '{registry}'"
            )));
        }

        // Split vendor/package so the dist URL segments are explicit.
        // The upload handler already validates the vendor/package format, so
        // a missing slash indicates a data integrity problem.
        let (vendor, pkg_name) = name.split_once('/').ok_or_else(|| {
            CoreError::Registry(format!("malformed composer package name: '{name}'"))
        })?;

        let base = base_url.trim_end_matches('/');
        let entries: Vec<serde_json::Value> = versions
            .iter()
            .filter_map(|pkg| {
                let mut entry = pkg.index_metadata.clone();
                let obj = entry.as_object_mut()?;
                // Inject/overwrite dist so downloads go through our proxy.
                obj.insert(
                    "dist".to_owned(),
                    serde_json::json!({
                        "type": "zip",
                        "url": format!(
                            "{base}/proxy/{registry}/dist/{vendor}/{pkg_name}/{version}",
                            version = pkg.version
                        ),
                        "shasum": pkg.checksum,
                    }),
                );
                obj.insert("name".to_owned(), serde_json::json!(name));
                obj.insert("version".to_owned(), serde_json::json!(pkg.version));
                obj.insert(
                    "time".to_owned(),
                    serde_json::json!(pkg.published_at.to_rfc3339()),
                );
                Some(entry)
            })
            .collect();

        if entries.is_empty() {
            return Err(CoreError::NotFound(format!(
                "composer package '{name}' has no valid versions in local registry '{registry}'"
            )));
        }

        Ok(serde_json::json!({
            "packages": { name: entries },
            "minified": "composer/2.0"
        }))
    }

    /// Return all distinct package names published in `registry`.
    /// Used to populate `available-packages` in `packages.json`.
    pub async fn get_composer_packages_list(
        &self,
        registry: &str,
    ) -> Result<Vec<String>, CoreError> {
        self.backend.list_package_names(registry).await
    }
}
