use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
    // ── Composer helpers ─────────────────────────────────────────────────────

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

    /// Return the (name, version) key for a locally published conda package
    /// whose `index_metadata.filename` matches `filename`, or `None` if not found.
    pub async fn find_conda_by_filename(
        &self,
        registry: &str,
        filename: &str,
    ) -> Result<Option<(String, String)>, CoreError> {
        let names = self.backend.list_package_names(registry).await?;
        for name in &names {
            let versions = self.backend.get_versions(registry, name).await?;
            for pkg in versions {
                let stored_filename = pkg
                    .index_metadata
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if stored_filename == filename {
                    return Ok(Some((pkg.name, pkg.version)));
                }
            }
        }
        Ok(None)
    }

    /// Build a PyPI Simple API HTML page listing all versions of `package_name`
    /// published in this local registry, formatted so `pip` can parse it.
    pub async fn get_pypi_simple_page(
        &self,
        registry: &str,
        package_name: &str,
        base_url: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, package_name, identity, "pypi package")
            .await?;
        let base = base_url.trim_end_matches('/');
        let mut links = String::new();
        for pkg in &versions {
            let filename = pkg
                .index_metadata
                .get("filename")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}-{}.tar.gz", pkg.name, pkg.version));
            let sha256 = pkg
                .index_metadata
                .get("sha256")
                .and_then(|v| v.as_str())
                .unwrap_or(&pkg.checksum);
            let url = format!("{base}/proxy/{registry}/packages/{filename}#sha256={sha256}");
            links.push_str(&format!("    <a href=\"{url}\">{filename}</a>\n"));
        }
        Ok(format!(
            "<!DOCTYPE html>\n<html>\n  <head><title>Links for {package_name}</title></head>\n  <body>\n    <h1>Links for {package_name}</h1>\n{links}  </body>\n</html>\n"
        ))
    }

    /// Build a conda `repodata.json`-compatible map for all locally published
    /// conda packages in `registry` whose `index_metadata.subdir` matches
    /// `platform` (e.g. `"linux-64"`, `"noarch"`).
    ///
    /// Returns a JSON object with `"packages"` and `"packages.conda"` keys
    /// ready to be serialised and served to conda clients.
    /// Returns `Some((filename, entry))` when `pkg` belongs to `platform`, or
    /// `None` when it should be excluded (wrong subdir or yanked).
    fn conda_repodata_entry(
        pkg: &crate::entities::PublishedPackage,
        platform: &str,
    ) -> Option<(String, serde_json::Value)> {
        let meta = &pkg.index_metadata;
        let subdir = meta.get("subdir").and_then(|v| v.as_str()).unwrap_or("");
        if !subdir.is_empty() && subdir != platform {
            return None;
        }
        let filename = match meta.get("filename").and_then(|v| v.as_str()) {
            Some(f) => f.to_owned(),
            None => {
                let build = meta.get("build").and_then(|v| v.as_str()).unwrap_or("0");
                format!("{}-{}-{}.tar.bz2", pkg.name, pkg.version, build)
            }
        };
        let mut entry = if meta.is_object() {
            meta.clone()
        } else {
            serde_json::json!({"name": pkg.name, "version": pkg.version})
        };
        if let Some(obj) = entry.as_object_mut() {
            obj.entry("sha256")
                .or_insert_with(|| serde_json::json!(pkg.checksum));
        }
        Some((filename, entry))
    }

    pub async fn get_conda_repodata(
        &self,
        registry: &str,
        platform: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let names = self.backend.list_package_names(registry).await?;

        let mut packages = serde_json::Map::new();
        let mut packages_conda = serde_json::Map::new();

        for name in &names {
            let versions = self.backend.get_versions(registry, name).await?;
            for pkg in versions.into_iter().filter(|p| !p.yanked) {
                let Some((filename, entry)) = Self::conda_repodata_entry(&pkg, platform) else {
                    continue;
                };
                if filename.ends_with(".conda") {
                    packages_conda.insert(filename, entry);
                } else {
                    packages.insert(filename, entry);
                }
            }
        }

        Ok(serde_json::json!({
            "packages": packages,
            "packages.conda": packages_conda,
        }))
    }
}
