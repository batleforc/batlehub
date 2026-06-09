use super::{
    artifact_storage_key, check_team_visibility, Bytes, CoreError, Identity, LocalRegistryService,
    PublishedPackage, Role, StreamExt, Visibility,
};

impl LocalRegistryService {
    /// Return the sparse index file content (newline-delimited JSON) for a Cargo crate.
    /// Returns `CoreError::NotFound` if the crate has never been published here.
    pub async fn get_index(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "crate '{}' not found in local registry '{}'",
                name, registry
            )));
        }
        let lines = versions
            .iter()
            .map(|v| serde_json::to_string(&v.index_metadata))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CoreError::Registry(e.to_string()))?;
        Ok(lines.join("\n"))
    }

    /// Build an npm packument for all published versions, rewriting `dist.tarball`
    /// to point at `base_url` (e.g. `"https://batlehub.example.com"`).
    pub async fn get_npm_packument(
        &self,
        registry: &str,
        name: &str,
        base_url: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "package '{}' not found in local registry '{}'",
                name, registry
            )));
        }

        let base = base_url.trim_end_matches('/');
        let mut versions_map = serde_json::Map::new();
        let mut time_map = serde_json::Map::new();
        let mut latest = String::new();

        for pkg in &versions {
            let mut meta = pkg.index_metadata.clone();
            if let Some(obj) = meta.as_object_mut() {
                let dist = obj.entry("dist").or_insert_with(|| serde_json::json!({}));
                if let Some(d) = dist.as_object_mut() {
                    d.insert(
                        "tarball".to_owned(),
                        serde_json::json!(format!(
                            "{base}/proxy/{registry}/{name}/{version}/tarball",
                            version = pkg.version
                        )),
                    );
                }
            }
            time_map.insert(
                pkg.version.clone(),
                serde_json::json!(pkg.published_at.to_rfc3339()),
            );
            versions_map.insert(pkg.version.clone(), meta);
            if !Self::is_prerelease(&pkg.version) {
                latest = pkg.version.clone();
            }
        }

        // When no stable version is visible, fall back to the newest pre-release so that
        // `dist-tags.latest` is always a valid (non-empty) version string.
        if latest.is_empty() {
            if let Some(p) = versions.last() {
                latest = p.version.clone();
            }
        }

        Ok(serde_json::json!({
            "name": name,
            "_id": name,
            "dist-tags": { "latest": latest },
            "versions": versions_map,
            "time": time_map
        }))
    }

    /// Return a single npm version metadata object with `dist.tarball` rewritten.
    pub async fn get_npm_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        base_url: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        self.check_visibility(registry, name, identity).await?;
        self.check_prerelease_access(registry, version, identity)
            .await?;
        let versions = self.backend.get_versions(registry, name).await?;
        let pkg = versions
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    name, version, registry
                ))
            })?;

        let base = base_url.trim_end_matches('/');
        let mut meta = pkg.index_metadata.clone();
        if let Some(obj) = meta.as_object_mut() {
            let dist = obj.entry("dist").or_insert_with(|| serde_json::json!({}));
            if let Some(d) = dist.as_object_mut() {
                d.insert(
                    "tarball".to_owned(),
                    serde_json::json!(format!("{base}/proxy/{registry}/{name}/{version}/tarball")),
                );
            }
        }
        Ok(meta)
    }

    /// Retrieve the raw artifact bytes for download.
    pub async fn get_artifact(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<Bytes, CoreError> {
        self.check_visibility(registry, name, identity).await?;
        let key = artifact_storage_key(registry, name, version);
        let artifact = self.storage.retrieve(&key).await?.ok_or_else(|| {
            CoreError::NotFound(format!(
                "{}/{}@{} not found in local registry",
                registry, name, version
            ))
        })?;
        let mut buf = Vec::new();
        let mut stream = artifact.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk?);
        }
        Ok(Bytes::from(buf))
    }

    // ── Visibility helpers ────────────────────────────────────────────────────

    /// Check whether `identity` is allowed to access `package` given its
    /// current visibility setting.
    ///
    /// - `Public`   → always allowed (even anonymous).
    /// - `Internal` → requires at least `Role::User`.
    /// - `Team`     → requires membership in the group that owns the namespace.
    ///   Falls back to `Internal` semantics when no claim exists.
    ///
    /// Admins bypass all checks. When no `team_namespace` port is configured,
    /// access is always permitted.
    pub async fn check_visibility(
        &self,
        registry: &str,
        package: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if identity.is_admin() {
            return Ok(());
        }
        let Some(ref ns_port) = self.team_namespace else {
            return Ok(());
        };
        let vis = ns_port.get_visibility(registry, package).await?;
        match vis {
            Visibility::Public => Ok(()),
            Visibility::Internal => {
                if identity.has_role_at_least(&Role::User) {
                    Ok(())
                } else {
                    Err(CoreError::AccessDenied(
                        "package is internal; authentication required".into(),
                    ))
                }
            }
            Visibility::Team => {
                check_team_visibility(&**ns_port, registry, package, identity).await
            }
        }
    }

    // ── Beta channel helpers ──────────────────────────────────────────────────

    /// Returns `true` when `version` is a pre-release.
    ///
    /// Handles semver pre-release components (`1.0.0-beta.1`), optional `v` prefixes
    /// (`v1.0.0-beta.1`), and Composer-style dev-branch aliases (`dev-main`, `1.x-dev`).
    pub(super) fn is_prerelease(version: &str) -> bool {
        // Composer dev-branch aliases are always pre-release.
        if version.starts_with("dev-") || version.ends_with("-dev") {
            return true;
        }
        // Strip optional leading 'v' before strict semver parse.
        let v = version.strip_prefix('v').unwrap_or(version);
        semver::Version::parse(v)
            .map(|sv| !sv.pre.is_empty())
            .unwrap_or(false)
    }

    /// Filter `versions` to remove pre-release entries when `identity` is not a
    /// beta-channel member and a beta channel is configured for `registry`.
    async fn filter_for_identity(
        &self,
        registry: &str,
        versions: Vec<PublishedPackage>,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let beta_port = self.hot.read().await.beta_channel.get(registry).cloned();
        let Some(beta_port) = beta_port else {
            return Ok(versions);
        };
        if beta_port.is_member(registry, identity).await? {
            return Ok(versions);
        }
        Ok(versions
            .into_iter()
            .filter(|p| !Self::is_prerelease(&p.version))
            .collect())
    }

    /// Convenience wrapper: `check_visibility` → `get_versions` → `filter_for_identity`.
    ///
    /// Returns the filtered list (may be empty — callers decide whether that is an error).
    pub(super) async fn load_visible_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        self.check_visibility(registry, name, identity).await?;
        let versions = self.backend.get_versions(registry, name).await?;
        self.filter_for_identity(registry, versions, identity).await
    }

    /// Returns `CoreError::NotFound` if `version` is a pre-release and the caller
    /// is not a beta-channel member for `registry`.
    pub async fn check_prerelease_access(
        &self,
        registry: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !Self::is_prerelease(version) {
            return Ok(());
        }
        let beta_port = self.hot.read().await.beta_channel.get(registry).cloned();
        let Some(beta_port) = beta_port else {
            return Ok(());
        };
        if beta_port.is_member(registry, identity).await? {
            return Ok(());
        }
        Err(CoreError::NotFound(format!(
            "version '{version}' is a pre-release and you are not a beta-channel member"
        )))
    }

    /// Look up the metadata for a specific published version.
    /// Returns `None` if not found (non-fatal — callers may skip signature headers).
    pub async fn get_version_meta(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Option<crate::entities::PublishedPackage> {
        self.backend
            .get_versions(registry, name)
            .await
            .ok()?
            .into_iter()
            .find(|p| p.version == version)
    }
}
