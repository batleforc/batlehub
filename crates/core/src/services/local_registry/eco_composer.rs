use super::{CoreError, Identity, LocalRegistryService};

/// Where the publish path stores the artifact's SHA-1, inside the package's
/// stored `index_metadata`.
///
/// Composer's `dist.shasum` is a SHA-1 and its client verifies against it, so
/// the SHA-256 this repository stores as *the* checksum cannot go in that
/// field. It is computed once at publish rather than per read: a p2 document
/// lists every version, and hashing each artifact to render it would make a
/// metadata request cost one storage read per version.
///
/// Underscore-prefixed and stripped before the entry is served — it is our
/// bookkeeping, not part of the package's `composer.json`.
pub const COMPOSER_DIST_SHA1: &str = "_batlehub_dist_sha1";

impl LocalRegistryService {
    /// Build a Packagist v2-compatible p2 JSON response for a locally published package.
    ///
    /// `base_url` is the registry's public base as seen by the requesting client
    /// (see [`LocalRegistryService::get_npm_packument`]).
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
                // The SHA-1 stored at publish. Absent for anything published
                // before this field existed, and then the `shasum` is omitted
                // rather than filled with the SHA-256 `pkg.checksum` holds:
                // Composer hashes the downloaded file with SHA-1 and compares,
                // so a SHA-256 there is not a weaker check, it is a failed
                // download every time.
                let dist_sha1 = obj
                    .remove(COMPOSER_DIST_SHA1)
                    .and_then(|v| v.as_str().map(str::to_owned));
                let mut dist = serde_json::json!({
                    "type": "zip",
                    "url": format!(
                        "{base}/dist/{vendor}/{pkg_name}/{version}",
                        version = pkg.version
                    ),
                });
                if let (Some(sha1), Some(dist_obj)) = (dist_sha1, dist.as_object_mut()) {
                    dist_obj.insert("shasum".to_owned(), serde_json::Value::String(sha1));
                }
                // Inject/overwrite dist so downloads go through our proxy.
                obj.insert("dist".to_owned(), dist);
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
