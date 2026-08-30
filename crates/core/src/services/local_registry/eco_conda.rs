use super::{CoreError, Identity, LocalRegistryService};

impl LocalRegistryService {
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

    /// `repodata.json` for `platform`, holding what `identity` may see.
    ///
    /// The identity is not decoration. This document names every package in the
    /// channel, and it used to be built from `backend.get_versions` directly —
    /// no visibility check, no grant filter — so a private conda package was
    /// listed to anyone who fetched the channel index, including callers the
    /// same registry answers `403` to on the package itself. That is survey
    /// finding 11's shape (a listing built from a bare name query) on the one
    /// ecosystem whose listing nobody had revisited, and RFC 0015 §4.4 is the
    /// rule it breaks.
    pub async fn get_conda_repodata(
        &self,
        registry: &str,
        platform: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let names = self.backend.list_package_names(registry).await?;
        let readable = self.readable_packages(registry, identity).await?;

        let mut packages = serde_json::Map::new();
        let mut packages_conda = serde_json::Map::new();

        for name in &names {
            if !readable.contains(name) {
                continue;
            }
            // `load_visible_versions` rather than `get_versions`: it applies the
            // visibility predicate and drops blocked versions, which is what the
            // per-package conda routes already do. A denial is a package this
            // caller does not see, not an error that blanks the channel.
            let versions = match self.load_visible_versions(registry, name, identity).await {
                Ok(v) => v,
                Err(CoreError::AccessDenied(_)) => continue,
                Err(e) => return Err(e),
            };
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
