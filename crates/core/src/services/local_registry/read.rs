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
        let versions = self
            .load_visible_versions_or_not_found(registry, name, identity, "crate")
            .await?;
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
        let versions = self
            .load_visible_versions_or_not_found(registry, name, identity, "package")
            .await?;

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
        super::validate_coordinate(name, version, None)?;
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
        let bytes = Bytes::from(buf);

        // Per-registry integrity (re-serve checksum) and signature policies.
        let (integrity, signing) = {
            let hot = self.hot.read().await;
            (
                hot.integrity.get(registry).cloned().unwrap_or_default(),
                hot.signing.get(registry).cloned().unwrap_or_default(),
            )
        };

        let verify_checksum = integrity.enabled && integrity.verify_on_serve;
        if verify_checksum || signing.verify_on_download {
            // Both checks need the stored per-version metadata (checksum + signature).
            // These are opt-in guarantees that every served byte is verified, so we
            // fail closed: a metadata-lookup error propagates, and a missing row for
            // bytes that exist in storage is an inconsistency we refuse to serve
            // unverified rather than silently skipping the check.
            let meta = self
                .backend
                .get_versions(registry, name)
                .await?
                .into_iter()
                .find(|p| p.version == version)
                .ok_or_else(|| {
                    CoreError::IntegrityFailure(format!(
                        "cannot verify {registry}/{name}@{version}: no published metadata found for stored artifact"
                    ))
                })?;
            if verify_checksum {
                self.verify_reserve_checksum(registry, name, version, &meta.checksum, &bytes)?;
            }
            if signing.verify_on_download {
                Self::verify_download_signature(
                    registry,
                    name,
                    version,
                    &signing,
                    meta.signature_bytes.as_deref(),
                    meta.signature_type.as_deref(),
                    &bytes,
                )?;
            }
        }

        Ok(bytes)
    }

    /// Re-verify stored bytes against the SHA-256 recorded at publish time.
    /// A mismatch means the stored artifact was corrupted or tampered with.
    fn verify_reserve_checksum(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        expected: &str,
        bytes: &Bytes,
    ) -> Result<(), CoreError> {
        use crate::services::integrity::{verify, IntegrityOutcome};
        match verify(expected, bytes) {
            IntegrityOutcome::Verified { algo } => {
                metrics::counter!("batlehub_integrity_checks_total", "registry" => registry.to_owned(), "outcome" => "verified", "phase" => "reserve").increment(1);
                tracing::debug!(
                    registry,
                    name,
                    version,
                    algo = algo.as_str(),
                    "local artifact re-verified on serve"
                );
                Ok(())
            }
            IntegrityOutcome::Mismatch {
                algo,
                expected,
                actual,
            } => {
                metrics::counter!("batlehub_integrity_checks_total", "registry" => registry.to_owned(), "outcome" => "mismatch", "phase" => "reserve").increment(1);
                tracing::warn!(registry, name, version, algo = algo.as_str(), %expected, %actual, "local artifact failed re-serve integrity check");
                Err(CoreError::IntegrityFailure(format!(
                    "stored artifact failed integrity check for {registry}/{name}@{version}: {} digest mismatch",
                    algo.as_str(),
                )))
            }
            IntegrityOutcome::Unparseable => {
                metrics::counter!("batlehub_integrity_checks_total", "registry" => registry.to_owned(), "outcome" => "unparseable", "phase" => "reserve").increment(1);
                tracing::warn!(
                    registry,
                    name,
                    version,
                    "stored checksum could not be parsed; serving without re-verification"
                );
                Ok(())
            }
        }
    }

    /// Verify a stored `ed25519` detached signature against the registry's
    /// trusted keys. Non-`ed25519` types and absent signatures are not verified
    /// here (publish-time `signing.required` governs presence).
    fn verify_download_signature(
        registry: &str,
        name: &str,
        version: &str,
        signing: &crate::services::hot_config::SigningConfig,
        sig_bytes: Option<&[u8]>,
        sig_type: Option<&str>,
        bytes: &Bytes,
    ) -> Result<(), CoreError> {
        use crate::services::signature::{verify_ed25519, ED25519_SIG_TYPE};
        let (Some(sig), Some(ty)) = (sig_bytes, sig_type) else {
            metrics::counter!("batlehub_signature_checks_total", "registry" => registry.to_owned(), "outcome" => "skipped").increment(1);
            return Ok(());
        };
        if !ty.eq_ignore_ascii_case(ED25519_SIG_TYPE) {
            // Only Ed25519 is verifiable here (rsa/PGP are banned); skip others.
            metrics::counter!("batlehub_signature_checks_total", "registry" => registry.to_owned(), "outcome" => "skipped").increment(1);
            return Ok(());
        }
        if verify_ed25519(&signing.trusted_keys, sig, bytes) {
            metrics::counter!("batlehub_signature_checks_total", "registry" => registry.to_owned(), "outcome" => "verified").increment(1);
            tracing::debug!(
                registry,
                name,
                version,
                "ed25519 artifact signature verified on download"
            );
            Ok(())
        } else {
            metrics::counter!("batlehub_signature_checks_total", "registry" => registry.to_owned(), "outcome" => "mismatch").increment(1);
            tracing::warn!(
                registry,
                name,
                version,
                "ed25519 artifact signature failed verification against trusted keys"
            );
            Err(CoreError::IntegrityFailure(format!(
                "artifact signature verification failed for {registry}/{name}@{version}"
            )))
        }
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

    /// `load_visible_versions`, returning `CoreError::NotFound` if the result is empty.
    ///
    /// `entity_label` is used in the error message, e.g. `"crate"`, `"gem"`, `"module"`.
    pub(super) async fn load_visible_versions_or_not_found(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
        entity_label: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "{entity_label} '{name}' not found in local registry '{registry}'"
            )));
        }
        Ok(versions)
    }

    /// Picks the newest non-prerelease version, falling back to the overall newest
    /// version if every entry is a pre-release. `versions` must be sorted ascending
    /// (oldest first), as returned by `load_visible_versions`.
    pub(super) fn latest_stable_or_newest(
        versions: &[PublishedPackage],
    ) -> Option<PublishedPackage> {
        versions
            .iter()
            .rev()
            .find(|v| !Self::is_prerelease(&v.version))
            .or_else(|| versions.last())
            .cloned()
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
