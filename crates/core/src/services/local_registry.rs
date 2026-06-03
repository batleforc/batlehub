use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;

use crate::{
    entities::{Identity, PublishedPackage, Role, SbomFormat, Visibility},
    error::CoreError,
    ports::{
        LocalRegistryBackend, OwnershipPort, StorageBackend, StorageMeta, TeamNamespacePort,
    },
    services::{
        explore_cache::ExploreCache,
        hot_config::{HotConfigLock, VersioningPolicy},
        quota::{QuotaCheck, QuotaService},
        sbom::{SbomPublishOptions, SbomService},
    },
};

// `VersioningPolicy` and `SigningConfig` are defined in hot_config and re-exported from services.

fn validate_version(version: &str, policy: &VersioningPolicy) -> Result<(), CoreError> {
    if policy.enforce_semver {
        match semver::Version::parse(version) {
            Err(_) => {
                return Err(CoreError::InvalidVersion(format!(
                    "version '{version}' is not valid semver"
                )));
            }
            Ok(sv) if !policy.allow_prerelease && !sv.pre.is_empty() => {
                return Err(CoreError::InvalidVersion(format!(
                    "pre-release versions are not allowed (got '{version}')"
                )));
            }
            Ok(_) => {}
        }
    }
    if let Some(ref re) = policy.version_pattern {
        if !re.is_match(version) {
            return Err(CoreError::InvalidVersion(format!(
                "version '{version}' does not match required pattern '{}'",
                re.as_str()
            )));
        }
    }
    Ok(())
}

/// Input to `LocalRegistryService::publish`.
pub struct PublishRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// Raw artifact bytes.
    pub artifact: Bytes,
    /// SHA-256 hex of `artifact`, computed by the caller (handler layer).
    pub checksum: String,
    /// Ecosystem-specific index metadata serialised as JSON.
    /// Cargo: serialised `CargoIndexEntry` (with `cksum` already set).
    /// npm: version metadata from the publish payload (`dist.tarball` stripped).
    /// VSIX: `{"id": "pub.name", "version": "1.0.0"}`.
    pub index_metadata: serde_json::Value,
    /// Identity of the publishing user.
    pub publisher: Identity,
    /// Raw signature bytes decoded from `X-Artifact-Signature` header, if present.
    pub signature_bytes: Option<Vec<u8>>,
    /// Signature type from `X-Signature-Type` header, if present.
    pub signature_type: Option<String>,
}

/// Authoritative local-registry service: publish, yank, index, artifact retrieval.
pub struct LocalRegistryService {
    pub backend: Arc<dyn LocalRegistryBackend>,
    pub storage: Arc<dyn StorageBackend>,
    /// Hot-swappable state (versioning, signing, beta_channel, size limit).
    pub hot: HotConfigLock,
    /// Optional publish quota enforcement. When `None`, quotas are disabled.
    pub quota: Option<Arc<QuotaService>>,
    /// Optional per-package ownership enforcement. When `None`, ownership is not enforced.
    pub ownership: Option<Arc<dyn OwnershipPort>>,
    /// Optional team namespace enforcement. When `None`, namespace gating is disabled.
    pub team_namespace: Option<Arc<dyn TeamNamespacePort>>,
    /// Optional SBOM service; when `None`, SBOM generation is disabled globally.
    pub sbom: Option<Arc<SbomService>>,
    /// Optional explore cache; invalidated automatically on successful publish.
    pub explore_cache: Option<Arc<ExploreCache>>,
}

/// OS/architecture pair identifying a specific Terraform provider binary.
#[derive(Debug, Clone, Copy)]
pub struct TerraformPlatform<'a> {
    pub os: &'a str,
    pub arch: &'a str,
}

impl LocalRegistryService {
    /// Validate and persist a published artifact.
    ///
    /// Returns a `QuotaCheck` describing the publisher's current quota state
    /// after the publish (useful for setting `X-Quota-*` response headers).
    /// Returns a zeroed `QuotaCheck` when no quota is configured.
    pub async fn publish(&self, req: PublishRequest) -> Result<QuotaCheck, CoreError> {
        if !req.publisher.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "publishing requires at least User role".into(),
            ));
        }

        // Snapshot hot-swappable policy (versioning, signing, size limit).
        let (versioning, signing, limit) = {
            let hot = self.hot.read().await;
            let versioning = hot.versioning.get(&req.registry).cloned();
            let signing = hot.signing.get(&req.registry).cloned();
            let limit = hot.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
            (versioning, signing, limit)
        };

        // Versioning policy check.
        if let Some(ref policy) = versioning {
            validate_version(&req.version, policy)?;
        }

        // Signing check.
        if let Some(ref signing) = signing {
            if signing.required && req.signature_bytes.is_none() {
                return Err(CoreError::AccessDenied(
                    "artifact signature required (X-Artifact-Signature header missing)".into(),
                ));
            }
            if !signing.allowed_types.is_empty() {
                if let Some(ref sig_type) = req.signature_type {
                    if !signing.allowed_types.iter().any(|t| t == sig_type) {
                        return Err(CoreError::AccessDenied(format!(
                            "signature type '{sig_type}' is not in the allowed list"
                        )));
                    }
                }
            }
        }

        // Namespace enforcement: if the package prefix is claimed by a team group,
        // only members of that group (or admins) may publish here.
        if let Some(ref ns_port) = self.team_namespace {
            if let Some(ns) = ns_port.find_namespace(&req.registry, &req.name).await? {
                let norm_id = ns.group_id.replace(' ', "");
                let ok = req.publisher.is_admin()
                    || req
                        .publisher
                        .groups
                        .iter()
                        .any(|g| g.replace(' ', "") == norm_id);
                if !ok {
                    return Err(CoreError::AccessDenied(format!(
                        "namespace '{}' in registry '{}' is owned by group '{}'; \
                         you are not a member",
                        ns.prefix, req.registry, ns.group_id
                    )));
                }
            }
        }

        // Ownership check: if ownership is configured and the package already exists,
        // verify the caller is a registered owner.
        let is_new_package = if let Some(ref ownership) = self.ownership {
            let package_exists = self.backend.exists(&req.registry, &req.name).await?;
            if package_exists
                && !ownership
                    .can_publish(&req.registry, &req.name, &req.publisher)
                    .await?
            {
                return Err(CoreError::AccessDenied(format!(
                    "you are not an owner of '{}' in registry '{}'",
                    req.name, req.registry
                )));
            }
            !package_exists
        } else {
            false
        };

        // `limit` was extracted from hot config above.
        if req.artifact.len() as u64 > limit {
            return Err(CoreError::PayloadTooLarge(format!(
                "artifact is {} bytes; limit is {}",
                req.artifact.len(),
                limit
            )));
        }

        // Check and record quota before persisting. This may return QuotaExceeded.
        let quota_check = if let Some(quota_svc) = &self.quota {
            quota_svc
                .check_and_record_publish(&req.publisher, &req.registry, req.artifact.len() as u64)
                .await?
        } else {
            QuotaCheck::default()
        };

        // Inherit the existing package visibility so that publishing a new version
        // doesn't silently reset a team/internal package back to public.
        // `get_visibility` returns Public when no published rows exist yet (first publish).
        // Propagate DB errors rather than defaulting to Public — silently publishing a
        // team-private package as world-readable during a DB outage is a security failure.
        let visibility = if let Some(ref ns_port) = self.team_namespace {
            ns_port.get_visibility(&req.registry, &req.name).await?
        } else {
            Visibility::default()
        };

        let pkg = PublishedPackage {
            registry: req.registry.clone(),
            name: req.name.clone(),
            version: req.version.clone(),
            checksum: req.checksum.clone(),
            yanked: false,
            index_metadata: req.index_metadata,
            published_at: chrono::Utc::now(),
            published_by: req.publisher.user_id.clone(),
            signature_bytes: req.signature_bytes.clone(),
            signature_type: req.signature_type.clone(),
            visibility,
        };

        let storage_key = artifact_storage_key(&req.registry, &req.name, &req.version);
        let bytes = req.artifact.len() as u64;

        // Step 1: reserve the version (inserted as 'pending', invisible to readers).
        if let Err(e) = self.backend.publish(pkg).await {
            // Row was not inserted; only quota needs rollback.
            self.revoke_quota(&req.publisher, &req.registry, bytes)
                .await;
            return Err(e);
        }

        // Step 2: persist artifact bytes. On failure, discard the pending row.
        if let Err(e) = self
            .storage
            .store(
                &storage_key,
                req.artifact.clone(),
                StorageMeta {
                    content_type: Some("application/octet-stream".into()),
                    size: None,
                    checksum: Some(req.checksum.clone()),
                },
            )
            .await
        {
            self.remove_pending(&req.registry, &req.name, &req.version)
                .await;
            self.revoke_quota(&req.publisher, &req.registry, bytes)
                .await;
            return Err(e);
        }

        // Step 3: promote the pending row to 'published'. On failure, undo both
        // the storage write and the pending row so the caller gets a clean error.
        if let Err(e) = self
            .backend
            .commit_publish(&req.registry, &req.name, &req.version)
            .await
        {
            self.remove_pending(&req.registry, &req.name, &req.version)
                .await;
            if let Err(err) = self.storage.delete(&storage_key).await {
                tracing::error!("storage cleanup after commit failure: {err}");
            }
            self.revoke_quota(&req.publisher, &req.registry, bytes)
                .await;
            return Err(e);
        }

        // Invalidate explore cache so the new version appears without waiting for TTL expiry.
        if let Some(ref cache) = self.explore_cache {
            cache.invalidate(Some(&req.registry)).await;
        }

        // Step 4: on first publish, register the publisher as the package admin.
        if is_new_package {
            if let (Some(ref ownership), Some(ref uid)) = (&self.ownership, &req.publisher.user_id)
            {
                if let Err(err) = ownership
                    .initialize_owner(&req.registry, &req.name, uid)
                    .await
                {
                    tracing::warn!("initialize_owner failed (non-fatal): {err}");
                }
            }
        }

        // Step 5: generate SBOM. When `required` is true and generation fails,
        // roll back the publish and return the error.
        if let Some(ref sbom_svc) = self.sbom {
            let sbom_cfg = {
                let hot = self.hot.read().await;
                hot.sbom.get(&req.registry).cloned()
            };
            if let Some(cfg) = sbom_cfg.filter(|c| c.enabled) {
                let formats: Vec<SbomFormat> = cfg
                    .formats
                    .iter()
                    .filter_map(|s| SbomFormat::parse(s))
                    .collect();
                let result = sbom_svc
                    .record_for_published(
                        &req.registry,
                        &req.name,
                        &req.version,
                        &storage_key,
                        &req.artifact,
                        SbomPublishOptions {
                            registry_type: &cfg.registry_type,
                            formats: &formats,
                            required: cfg.required,
                        },
                    )
                    .await;
                match result {
                    Err(e) if cfg.required => {
                        self.remove_pending(&req.registry, &req.name, &req.version)
                            .await;
                        if let Err(err) = self.storage.delete(&storage_key).await {
                            tracing::error!("storage cleanup after sbom failure: {err}");
                        }
                        self.revoke_quota(&req.publisher, &req.registry, bytes).await;
                        return Err(e);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "sbom generation failed (non-fatal)");
                    }
                    Ok(()) => {}
                }
            }
        }

        Ok(quota_check)
    }

    pub async fn yank(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "yank requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.backend.yank(registry, name, version).await
    }

    pub async fn unyank(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "unyank requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.backend.unyank(registry, name, version).await
    }

    /// If a namespace claim covers `package` in `registry`, verify `identity` is
    /// a member of the owning group. Admins and unclaimed packages bypass this.
    pub async fn check_namespace_membership(
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
        if let Some(ns) = ns_port.find_namespace(registry, package).await? {
            let norm_id = ns.group_id.replace(' ', "");
            let ok = identity
                .groups
                .iter()
                .any(|g| g.replace(' ', "") == norm_id);
            if !ok {
                return Err(CoreError::AccessDenied(format!(
                    "namespace '{}' in registry '{}' is owned by group '{}'; \
                     you are not a member",
                    ns.prefix, registry, ns.group_id
                )));
            }
        }
        Ok(())
    }

    async fn remove_pending(&self, registry: &str, name: &str, version: &str) {
        if let Err(err) = self.backend.remove_version(registry, name, version).await {
            tracing::error!("pending row cleanup failed: {err}");
        }
    }

    async fn revoke_quota(&self, identity: &Identity, registry: &str, bytes: u64) {
        if let Some(svc) = &self.quota {
            if let Err(err) = svc.revoke_publish(identity, registry, bytes).await {
                tracing::error!("quota revoke failed: {err}");
            }
        }
    }

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
                match ns_port.find_namespace(registry, package).await? {
                    Some(ns)
                        if identity
                            .groups
                            .iter()
                            .any(|g| g.replace(' ', "") == ns.group_id.replace(' ', "")) =>
                    {
                        Ok(())
                    }
                    Some(ns) => Err(CoreError::AccessDenied(format!(
                        "package visibility is 'team'; must be a member of group '{}'",
                        ns.group_id
                    ))),
                    // No claim found: deny everyone. Falling back to "any authenticated user"
                    // would allow non-team members to read team-private packages whenever
                    // the namespace claim is missing or has been deleted.
                    None => Err(CoreError::AccessDenied(
                        "package visibility is 'team' but no namespace claim is configured; \
                         access denied"
                            .into(),
                    )),
                }
            }
        }
    }

    // ── Beta channel helpers ──────────────────────────────────────────────────

    /// Returns `true` when `version` is a pre-release.
    ///
    /// Handles semver pre-release components (`1.0.0-beta.1`), optional `v` prefixes
    /// (`v1.0.0-beta.1`), and Composer-style dev-branch aliases (`dev-main`, `1.x-dev`).
    fn is_prerelease(version: &str) -> bool {
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
    async fn load_visible_versions(
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

    /// Return newline-delimited version list for a locally published Go module.
    /// Returns `CoreError::NotFound` if the module has never been published here.
    pub async fn get_go_version_list(
        &self,
        registry: &str,
        module: &str,
        identity: &Identity,
    ) -> Result<String, CoreError> {
        let versions = self.load_visible_versions(registry, module, identity).await?;
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
        let versions = self.load_visible_versions(registry, module, identity).await?;
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

    /// Return the `/api/v1/gems/{name}.json`-compatible info for the latest gem version.
    pub async fn get_rubygems_gem_info(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        let latest = versions
            .iter()
            .rev()
            .find(|v| !Self::is_prerelease(&v.version))
            .or_else(|| versions.last())
            .cloned()
            .ok_or_else(|| {
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
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "gem '{name}' not found in local registry '{registry}'"
            )));
        }
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

    /// Return all non-empty published versions for a Maven artifact (`groupId:artifactId`).
    /// Returns `CoreError::NotFound` when none are published.
    pub async fn get_maven_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "artifact '{name}' not found in local registry '{registry}'"
            )));
        }
        Ok(versions)
    }

    /// Return all locally published versions of a NuGet package.
    pub async fn get_nuget_versions(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "NuGet package '{name}' not found in local registry '{registry}'"
            )));
        }
        Ok(versions)
    }

    /// Build the Terraform module versions envelope from `local_packages`.
    pub async fn get_tf_module_versions_response(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "module '{name}' not found in local registry '{registry}'"
            )));
        }
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
        let versions = self.load_visible_versions(registry, name, identity).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "provider '{name}' not found in local registry '{registry}'"
            )));
        }
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
            .load_visible_versions(registry, package_name, identity)
            .await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "pypi package '{package_name}' not found in local registry '{registry}'"
            )));
        }
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
                let meta = &pkg.index_metadata;
                // Filter by platform: accept packages whose subdir matches or where
                // subdir is absent (treat as matching any platform query).
                let subdir = meta.get("subdir").and_then(|v| v.as_str()).unwrap_or("");
                if !subdir.is_empty() && subdir != platform {
                    continue;
                }
                let filename = match meta.get("filename").and_then(|v| v.as_str()) {
                    Some(f) => f.to_owned(),
                    None => {
                        // Reconstruct filename from name/version/build
                        let build = meta
                            .get("build")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0");
                        format!("{}-{}-{}.tar.bz2", pkg.name, pkg.version, build)
                    }
                };
                // Build the repodata entry from index_metadata plus checksum.
                let mut entry = if meta.is_object() {
                    meta.clone()
                } else {
                    serde_json::json!({
                        "name": pkg.name,
                        "version": pkg.version,
                    })
                };
                // Ensure sha256 is set from the stored checksum if not in metadata.
                if let Some(obj) = entry.as_object_mut() {
                    obj.entry("sha256")
                        .or_insert_with(|| serde_json::json!(pkg.checksum));
                }

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

/// Stable storage key for a locally published artifact.
/// Distinct from the proxy `artifact:…` namespace to avoid collisions.
pub fn artifact_storage_key(registry: &str, name: &str, version: &str) -> String {
    format!("local:{}/{}/{}", registry, name, version)
}

/// Storage key for a non-POM Maven artifact (jar, checksum, etc.).
/// Multiple artifact files can coexist under the same version.
pub fn maven_artifact_storage_key(
    registry: &str,
    name: &str,
    version: &str,
    filename: &str,
) -> String {
    format!("local:{}/{}/{}/{}", registry, name, version, filename)
}

/// Storage key for a Terraform provider platform binary.
pub fn tf_provider_binary_storage_key(
    registry: &str,
    namespace: &str,
    ptype: &str,
    version: &str,
    os: &str,
    arch: &str,
) -> String {
    format!("local:{registry}/providers/{namespace}/{ptype}/{version}/{os}-{arch}")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use chrono::Utc;

    use super::*;
    use crate::{
        entities::{Identity, Role},
        error::CoreError,
        ports::{StorageBackend, StorageMeta, StoredArtifact},
        services::hot_config::{new_hot_lock, HotConfig},
    };

    // ── Minimal mock backend ──────────────────────────────────────────────────

    #[derive(Default)]
    struct InMemBackend {
        versions: Mutex<Vec<PublishedPackage>>,
    }

    impl InMemBackend {
        fn arc() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn seed(&self, pkg: PublishedPackage) {
            self.versions.lock().unwrap().push(pkg);
        }
    }

    #[async_trait]
    impl crate::ports::LocalRegistryBackend for InMemBackend {
        async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
            self.versions.lock().unwrap().push(pkg);
            Ok(())
        }
        async fn yank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn unyank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn get_versions(
            &self,
            registry: &str,
            name: &str,
        ) -> Result<Vec<PublishedPackage>, CoreError> {
            Ok(self
                .versions
                .lock()
                .unwrap()
                .iter()
                .filter(|p| p.registry == registry && p.name == name)
                .cloned()
                .collect())
        }
        async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
            Ok(self
                .versions
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.registry == registry && p.name == name))
        }
    }

    struct NoopStorage;

    #[async_trait]
    impl StorageBackend for NoopStorage {
        async fn store(&self, _: &str, _: Bytes, _: StorageMeta) -> Result<(), CoreError> {
            Ok(())
        }
        async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> {
            Ok(None)
        }
        async fn exists(&self, _: &str) -> Result<bool, CoreError> {
            Ok(false)
        }
        async fn delete(&self, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> {
            Ok(0)
        }
        async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> {
            Ok((0, 0))
        }
        async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> {
            Ok(vec![])
        }
    }

    fn svc(backend: Arc<InMemBackend>, max_bytes: Option<u64>) -> LocalRegistryService {
        LocalRegistryService {
            backend,
            storage: Arc::new(NoopStorage),
            hot: new_hot_lock(HotConfig {
                registries: HashMap::new(),
                policies: HashMap::new(),
                versioning: HashMap::new(),
                signing: HashMap::new(),
                sbom: HashMap::new(),
                beta_channel: HashMap::new(),
                max_artifact_size_bytes: max_bytes,
            }),
            quota: None,
            ownership: None,
            team_namespace: None,
            sbom: None,
            explore_cache: None,
        }
    }

    fn pkg(registry: &str, name: &str, version: &str) -> PublishedPackage {
        PublishedPackage {
            registry: registry.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            checksum: "abc".to_owned(),
            yanked: false,
            index_metadata: serde_json::json!({}),
            published_at: Utc::now(),
            published_by: None,
            signature_bytes: None,
            signature_type: None,
            visibility: Default::default(),
        }
    }

    fn anon() -> Identity {
        Identity {
            user_id: None,
            role: Role::Anonymous,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn user() -> Identity {
        Identity {
            user_id: Some("u1".into()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        }
    }

    // ── publish error paths ───────────────────────────────────────────────────

    #[tokio::test]
    async fn publish_rejects_oversized_artifact() {
        let backend = InMemBackend::arc();
        let s = svc(backend, Some(10)); // 10-byte limit
        let req = PublishRequest {
            registry: "npm".into(),
            name: "big".into(),
            version: "1.0.0".into(),
            artifact: Bytes::from(vec![0u8; 11]), // 11 bytes > 10-byte limit
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: user(),
            signature_bytes: None,
            signature_type: None,
        };
        let err = s.publish(req).await.unwrap_err();
        assert!(matches!(err, CoreError::PayloadTooLarge(_)));
    }

    // ── yank / unyank role checks ─────────────────────────────────────────────

    #[tokio::test]
    async fn yank_requires_user_role() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .yank("cargo", "serde", "1.0.0", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn unyank_requires_user_role() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .unyank("cargo", "serde", "1.0.0", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    // ── npm packument / version not-found ─────────────────────────────────────

    #[tokio::test]
    async fn get_npm_packument_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_npm_packument("npm", "unknown", "http://localhost", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_npm_version_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("npm", "express", "4.0.0"));
        let s = svc(backend, None);
        let err = s
            .get_npm_version("npm", "express", "9.9.9", "http://localhost", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // ── go module not-found ───────────────────────────────────────────────────

    #[tokio::test]
    async fn get_go_version_list_not_found_when_empty() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_go_version_list("go", "example.com/mod", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_info_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s
            .get_go_info("go", "example.com/mod", "v9.9.9", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_mod_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s
            .get_go_mod("go", "example.com/mod", "v9.9.9", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_mod_not_found_when_no_go_mod_key() {
        let backend = InMemBackend::arc();
        // Package exists but index_metadata has no "go_mod" key
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s
            .get_go_mod("go", "example.com/mod", "v1.0.0", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_latest_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_go_latest("go", "example.com/mod", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // ── maven / nuget / pypi / composer not-found ────────────────────────────────

    #[tokio::test]
    async fn get_maven_versions_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_maven_versions("maven", "com.example:mylib", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_nuget_versions_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_nuget_versions("nuget", "Newtonsoft.Json", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_nuget_versions_returns_versions_when_published() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("nuget", "mylib", "1.0.0"));
        backend.seed(pkg("nuget", "mylib", "2.0.0"));
        let s = svc(backend, None);
        let versions = s
            .get_nuget_versions("nuget", "mylib", &anon())
            .await
            .unwrap();
        assert_eq!(versions.len(), 2);
    }

    #[tokio::test]
    async fn get_pypi_simple_page_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_pypi_simple_page("pypi", "requests", "http://localhost", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_composer_p2_response_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s
            .get_composer_p2_response("composer", "vendor/pkg", "http://localhost", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // ── Beta channel ─────────────────────────────────────────────────────────────

    /// Minimal in-memory BetaChannelPort whose membership set is seeded at construction.
    struct MemBetaChannel {
        members: std::collections::HashSet<String>, // user_ids
    }

    impl MemBetaChannel {
        fn with_users(ids: &[&str]) -> Arc<Self> {
            Arc::new(Self {
                members: ids.iter().map(|s| s.to_string()).collect(),
            })
        }
        fn empty() -> Arc<Self> {
            Arc::new(Self {
                members: std::collections::HashSet::new(),
            })
        }
    }

    #[async_trait]
    impl crate::ports::BetaChannelPort for MemBetaChannel {
        async fn is_member(&self, _registry: &str, identity: &Identity) -> Result<bool, CoreError> {
            Ok(identity
                .user_id
                .as_ref()
                .map(|id| self.members.contains(id))
                .unwrap_or(false))
        }
        async fn add_member(
            &self,
            _: &str,
            _: crate::ports::BetaChannelEntry,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn remove_member(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_members(
            &self,
            _: &str,
        ) -> Result<Vec<crate::ports::BetaChannelEntry>, CoreError> {
            Ok(vec![])
        }
    }

    fn svc_with_beta(
        backend: Arc<InMemBackend>,
        beta: Arc<dyn crate::ports::BetaChannelPort>,
    ) -> LocalRegistryService {
        let mut bc = HashMap::new();
        bc.insert("reg".to_owned(), beta as Arc<dyn crate::ports::BetaChannelPort>);
        LocalRegistryService {
            backend,
            storage: Arc::new(NoopStorage),
            hot: new_hot_lock(HotConfig {
                registries: HashMap::new(),
                policies: HashMap::new(),
                versioning: HashMap::new(),
                signing: HashMap::new(),
                sbom: HashMap::new(),
                beta_channel: bc,
                max_artifact_size_bytes: None,
            }),
            quota: None,
            ownership: None,
            team_namespace: None,
            sbom: None,
            explore_cache: None,
        }
    }

    fn beta_user() -> Identity {
        Identity {
            user_id: Some("beta".into()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        }
    }

    // No beta channel configured → all versions visible to everyone (tested via npm packument).
    #[tokio::test]
    async fn filter_no_beta_channel_shows_all_versions() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "lib", "1.0.0"));
        backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
        let s = svc(backend, None);
        let doc = s
            .get_npm_packument("reg", "lib", "http://localhost", &anon())
            .await
            .unwrap();
        assert_eq!(doc["versions"].as_object().unwrap().len(), 2);
    }

    // Beta channel configured; anonymous user sees only stable versions.
    #[tokio::test]
    async fn filter_non_member_hides_prerelease() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "lib", "1.0.0"));
        backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
        let s = svc_with_beta(backend, MemBetaChannel::empty());
        let doc = s
            .get_npm_packument("reg", "lib", "http://localhost", &anon())
            .await
            .unwrap();
        let versions = doc["versions"].as_object().unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions.contains_key("1.0.0"));
    }

    // Beta channel configured; member sees all versions including pre-release.
    #[tokio::test]
    async fn filter_member_sees_prerelease() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "lib", "1.0.0"));
        backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let doc = s
            .get_npm_packument("reg", "lib", "http://localhost", &beta_user())
            .await
            .unwrap();
        assert_eq!(doc["versions"].as_object().unwrap().len(), 2);
    }

    // check_prerelease_access passes for stable versions regardless of membership.
    #[tokio::test]
    async fn check_prerelease_access_stable_always_ok() {
        let backend = InMemBackend::arc();
        let s = svc_with_beta(backend, MemBetaChannel::empty());
        s.check_prerelease_access("reg", "1.0.0", &anon())
            .await
            .unwrap();
    }

    // check_prerelease_access blocks non-members on pre-release versions.
    #[tokio::test]
    async fn check_prerelease_access_blocks_non_member() {
        let backend = InMemBackend::arc();
        let s = svc_with_beta(backend, MemBetaChannel::empty());
        let err = s
            .check_prerelease_access("reg", "1.1.0-beta.1", &anon())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // check_prerelease_access allows members on pre-release versions.
    #[tokio::test]
    async fn check_prerelease_access_allows_member() {
        let backend = InMemBackend::arc();
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        s.check_prerelease_access("reg", "1.1.0-beta.1", &beta_user())
            .await
            .unwrap();
    }

    // check_prerelease_access passes when no beta channel is configured (open access).
    #[tokio::test]
    async fn check_prerelease_access_no_channel_open() {
        let backend = InMemBackend::arc();
        let s = svc(backend, None);
        s.check_prerelease_access("reg", "1.1.0-beta.1", &anon())
            .await
            .unwrap();
    }

    // npm packument: dist-tags.latest must point to latest stable, not pre-release.
    #[tokio::test]
    async fn npm_packument_latest_tag_skips_prerelease() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "pkg", "1.0.0"));
        backend.seed(pkg("reg", "pkg", "2.0.0-alpha.1"));
        // Even beta members should not see a pre-release as `latest`.
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let doc = s
            .get_npm_packument("reg", "pkg", "http://localhost", &beta_user())
            .await
            .unwrap();
        let latest = doc["dist-tags"]["latest"].as_str().unwrap();
        assert_eq!(latest, "1.0.0");
    }

    // npm packument: if all visible versions are pre-release, latest falls back to the newest pre-release.
    #[tokio::test]
    async fn npm_packument_latest_tag_only_prereleases() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "pkg", "1.0.0-beta.1"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let doc = s
            .get_npm_packument("reg", "pkg", "http://localhost", &beta_user())
            .await
            .unwrap();
        // No stable version; latest must fall back to the newest pre-release, not "".
        let latest = doc["dist-tags"]["latest"].as_str().unwrap();
        assert_eq!(latest, "1.0.0-beta.1");
    }

    // go @latest: prefers last stable; falls back to last pre-release only if no stable exists.
    #[tokio::test]
    async fn go_latest_prefers_stable_over_prerelease() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "mod", "1.0.0"));
        backend.seed(pkg("reg", "mod", "2.0.0-rc.1"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let info = s.get_go_latest("reg", "mod", &beta_user()).await.unwrap();
        assert_eq!(info["Version"].as_str().unwrap(), "1.0.0");
    }

    #[tokio::test]
    async fn go_latest_falls_back_to_prerelease_when_all_prerelease() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "mod", "1.0.0-alpha.1"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let info = s.get_go_latest("reg", "mod", &beta_user()).await.unwrap();
        assert_eq!(info["Version"].as_str().unwrap(), "1.0.0-alpha.1");
    }

    // rubygems gem_info: same stable-preference behaviour.
    #[tokio::test]
    async fn rubygems_gem_info_prefers_stable() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "gem", "1.0.0"));
        backend.seed(pkg("reg", "gem", "1.1.0-pre"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let info = s
            .get_rubygems_gem_info("reg", "gem", &beta_user())
            .await
            .unwrap();
        assert_eq!(info["version"].as_str().unwrap(), "1.0.0");
    }

    // rubygems versions: prerelease field uses semver-aware detection.
    #[tokio::test]
    async fn rubygems_versions_prerelease_flag_uses_semver() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "gem", "1.0.0"));
        backend.seed(pkg("reg", "gem", "1.1.0-rc.1"));
        let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
        let versions = s
            .get_rubygems_versions("reg", "gem", &beta_user())
            .await
            .unwrap();
        // Newest first; 1.1.0-rc.1 is index 0.
        let pre = versions[0]["prerelease"].as_bool().unwrap();
        let stable = versions[1]["prerelease"].as_bool().unwrap();
        assert!(pre, "1.1.0-rc.1 should be marked prerelease=true");
        assert!(!stable, "1.0.0 should be marked prerelease=false");
    }

    // is_prerelease handles v-prefixed and Composer dev-branch versions.
    #[test]
    fn is_prerelease_handles_v_prefix_and_dev_branches() {
        let check = |v: &str| LocalRegistryService::is_prerelease(v);
        assert!(check("v1.0.0-beta.1"), "v-prefixed pre-release");
        assert!(check("dev-main"), "dev- prefix");
        assert!(check("dev-feature/branch"), "dev- with path");
        assert!(check("1.0.0-dev"), "-dev suffix");
        assert!(!check("v1.0.0"), "v-prefixed stable");
        assert!(!check("1.0.0"), "plain stable");
        assert!(!check("1.0.0.0"), "four-part (non-semver stable)");
    }

    // check_prerelease_access blocks non-members on Composer dev-branch versions.
    #[tokio::test]
    async fn check_prerelease_access_blocks_dev_branch_non_member() {
        let backend = InMemBackend::arc();
        let s = svc_with_beta(backend, MemBetaChannel::empty());
        let err = s
            .check_prerelease_access("reg", "dev-main", &anon())
            .await
            .unwrap_err();
        assert!(
            matches!(err, CoreError::NotFound(_)),
            "dev-main must be gated"
        );
    }

    // ── Team namespace enforcement tests ─────────────────────────────────────

    #[derive(Debug, Default)]
    struct MockTeamNamespace {
        namespaces: Mutex<Vec<crate::entities::TeamNamespace>>,
        visibility: Mutex<HashMap<(String, String), Visibility>>,
    }

    impl MockTeamNamespace {
        fn arc() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn with_namespace(registry: &str, prefix: &str, group: &str) -> Arc<Self> {
            let s = Self::arc();
            s.namespaces
                .lock()
                .unwrap()
                .push(crate::entities::TeamNamespace {
                    registry: registry.to_owned(),
                    prefix: prefix.to_owned(),
                    group_id: group.to_owned(),
                    claimed_by: None,
                });
            s
        }
        fn with_visibility(registry: &str, package: &str, vis: Visibility) -> Arc<Self> {
            let s = Self::arc();
            s.visibility
                .lock()
                .unwrap()
                .insert((registry.to_owned(), package.to_owned()), vis);
            s
        }
    }

    #[async_trait]
    impl TeamNamespacePort for MockTeamNamespace {
        async fn find_namespace(
            &self,
            registry: &str,
            package: &str,
        ) -> Result<Option<crate::entities::TeamNamespace>, CoreError> {
            let ns = self.namespaces.lock().unwrap();
            let result = ns
                .iter()
                .filter(|n| {
                    n.registry == registry
                        && (package == n.prefix
                            || (package.len() > n.prefix.len()
                                && package[..n.prefix.len() + 1] == format!("{}/", n.prefix)))
                })
                .max_by_key(|n| n.prefix.len())
                .cloned();
            Ok(result)
        }
        async fn list_namespaces(
            &self,
            _: &str,
        ) -> Result<Vec<crate::entities::TeamNamespace>, CoreError> {
            Ok(vec![])
        }
        async fn claim_namespace(
            &self,
            _: crate::entities::TeamNamespace,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn release_namespace(&self, _: &str, _: &str) -> Result<(), CoreError> {
            Ok(())
        }
        async fn set_visibility(&self, _: &str, _: &str, _: Visibility) -> Result<(), CoreError> {
            Ok(())
        }
        async fn get_visibility(
            &self,
            registry: &str,
            package: &str,
        ) -> Result<Visibility, CoreError> {
            Ok(self
                .visibility
                .lock()
                .unwrap()
                .get(&(registry.to_owned(), package.to_owned()))
                .cloned()
                .unwrap_or_default())
        }
        async fn list_namespaces_for_groups(
            &self,
            groups: &[String],
        ) -> Result<Vec<crate::entities::TeamNamespace>, CoreError> {
            let ns = self.namespaces.lock().unwrap();
            Ok(ns
                .iter()
                .filter(|n| {
                    groups
                        .iter()
                        .any(|g| g.replace(' ', "") == n.group_id.replace(' ', ""))
                })
                .cloned()
                .collect())
        }
        async fn list_packages_in_namespace(
            &self,
            _: &str,
            _: &str,
            _: u64,
            _: u64,
        ) -> Result<Vec<crate::entities::NamespacePackage>, CoreError> {
            Ok(vec![])
        }
    }

    fn svc_with_ns(
        backend: Arc<InMemBackend>,
        ns: Arc<dyn TeamNamespacePort>,
    ) -> LocalRegistryService {
        LocalRegistryService {
            backend,
            storage: Arc::new(NoopStorage),
            hot: new_hot_lock(HotConfig {
                registries: HashMap::new(),
                policies: HashMap::new(),
                versioning: HashMap::new(),
                signing: HashMap::new(),
                sbom: HashMap::new(),
                beta_channel: HashMap::new(),
                max_artifact_size_bytes: None,
            }),
            quota: None,
            ownership: None,
            team_namespace: Some(ns),
            sbom: None,
            explore_cache: None,
        }
    }

    fn member() -> Identity {
        Identity {
            user_id: Some("m1".into()),
            role: Role::User,
            auth_provider: None,
            groups: vec!["team-a".into()],
        }
    }

    fn non_member() -> Identity {
        Identity {
            user_id: Some("u2".into()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn admin_id() -> Identity {
        Identity {
            user_id: Some("adm".into()),
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        }
    }

    #[tokio::test]
    async fn namespace_enforcement_blocks_non_member() {
        let backend = InMemBackend::arc();
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        let req = PublishRequest {
            registry: "reg".into(),
            name: "frontend/utils".into(),
            version: "1.0.0".into(),
            artifact: Bytes::from("data"),
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: non_member(),
            signature_bytes: None,
            signature_type: None,
        };
        let err = s.publish(req).await.unwrap_err();
        assert!(
            matches!(err, CoreError::AccessDenied(_)),
            "non-member must be denied"
        );
    }

    #[tokio::test]
    async fn namespace_enforcement_allows_member() {
        let backend = InMemBackend::arc();
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        let req = PublishRequest {
            registry: "reg".into(),
            name: "frontend/utils".into(),
            version: "1.0.0".into(),
            artifact: Bytes::from("data"),
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: member(),
            signature_bytes: None,
            signature_type: None,
        };
        assert!(s.publish(req).await.is_ok(), "member must be allowed");
    }

    #[tokio::test]
    async fn namespace_enforcement_admin_bypasses() {
        let backend = InMemBackend::arc();
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        let req = PublishRequest {
            registry: "reg".into(),
            name: "frontend/utils".into(),
            version: "1.0.0".into(),
            artifact: Bytes::from("data"),
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: admin_id(),
            signature_bytes: None,
            signature_type: None,
        };
        assert!(
            s.publish(req).await.is_ok(),
            "admin must bypass namespace gate"
        );
    }

    #[tokio::test]
    async fn no_namespace_claim_allows_any_user() {
        let backend = InMemBackend::arc();
        let ns = MockTeamNamespace::arc(); // no namespaces
        let s = svc_with_ns(backend, ns);
        let req = PublishRequest {
            registry: "reg".into(),
            name: "any/package".into(),
            version: "1.0.0".into(),
            artifact: Bytes::from("data"),
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: non_member(),
            signature_bytes: None,
            signature_type: None,
        };
        assert!(
            s.publish(req).await.is_ok(),
            "unclaimed namespace allows any user"
        );
    }

    // ── check_visibility tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn visibility_public_allows_anonymous() {
        let s = svc(InMemBackend::arc(), None);
        // no team_namespace configured -> always Ok
        assert!(s.check_visibility("reg", "pkg", &anon()).await.is_ok());
    }

    #[tokio::test]
    async fn visibility_internal_blocks_anonymous() {
        let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Internal);
        let s = svc_with_ns(InMemBackend::arc(), ns);
        let err = s.check_visibility("reg", "pkg", &anon()).await.unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn visibility_internal_allows_user() {
        let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Internal);
        let s = svc_with_ns(InMemBackend::arc(), ns);
        assert!(s
            .check_visibility("reg", "pkg", &non_member())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn visibility_team_blocks_non_member() {
        let mock = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        // override visibility map
        let mock = {
            let inner = Arc::try_unwrap(mock).unwrap();
            inner
                .visibility
                .lock()
                .unwrap()
                .insert(("reg".into(), "frontend/pkg".into()), Visibility::Team);
            Arc::new(inner)
        };
        let s = svc_with_ns(InMemBackend::arc(), mock);
        let err = s
            .check_visibility("reg", "frontend/pkg", &non_member())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn visibility_team_allows_member() {
        let mock = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        mock.visibility
            .lock()
            .unwrap()
            .insert(("reg".into(), "frontend/pkg".into()), Visibility::Team);
        let s = svc_with_ns(InMemBackend::arc(), mock);
        assert!(s
            .check_visibility("reg", "frontend/pkg", &member())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn visibility_admin_bypasses_all() {
        let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
        let s = svc_with_ns(InMemBackend::arc(), ns);
        assert!(s.check_visibility("reg", "pkg", &admin_id()).await.is_ok());
    }

    // When Team visibility is set but no namespace claim exists, access must
    // be denied for ALL non-admins — falling back to "any authenticated user"
    // would allow non-team members to read team-private packages.
    #[tokio::test]
    async fn visibility_team_no_claim_denies_authenticated_user() {
        // Visibility is Team but no namespace claim is seeded.
        let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
        let s = svc_with_ns(InMemBackend::arc(), ns);
        let err = s
            .check_visibility("reg", "pkg", &non_member())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn visibility_team_no_claim_denies_anonymous() {
        let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
        let s = svc_with_ns(InMemBackend::arc(), ns);
        let err = s.check_visibility("reg", "pkg", &anon()).await.unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    // Verify visibility is inherited when a second version is published on a
    // package that already has a non-public visibility.
    #[tokio::test]
    async fn publish_second_version_inherits_visibility() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "my-pkg", "1.0.0"));

        // Seed visibility = Internal for the first version.
        let ns = MockTeamNamespace::arc();
        ns.visibility
            .lock()
            .unwrap()
            .insert(("reg".into(), "my-pkg".into()), Visibility::Internal);
        let s = svc_with_ns(backend, ns);

        let req = PublishRequest {
            registry: "reg".into(),
            name: "my-pkg".into(),
            version: "2.0.0".into(),
            artifact: bytes::Bytes::from("data"),
            checksum: "abc".into(),
            index_metadata: serde_json::json!({}),
            publisher: user(),
            signature_bytes: None,
            signature_type: None,
        };
        s.publish(req).await.unwrap();

        // The newly published version must carry the inherited visibility.
        let versions = s.backend.get_versions("reg", "my-pkg").await.unwrap();
        let v2 = versions.iter().find(|v| v.version == "2.0.0").unwrap();
        assert_eq!(
            v2.visibility,
            Visibility::Internal,
            "second version must inherit Internal visibility from the package"
        );
    }

    // ── yank/unyank namespace enforcement ────────────────────────────────────

    #[tokio::test]
    async fn yank_blocks_non_member_in_claimed_namespace() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        let err = s
            .yank("reg", "frontend/utils", "1.0.0", &non_member())
            .await
            .unwrap_err();
        assert!(
            matches!(err, CoreError::AccessDenied(_)),
            "non-member must not yank namespace package"
        );
    }

    #[tokio::test]
    async fn yank_allows_namespace_member() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        assert!(s
            .yank("reg", "frontend/utils", "1.0.0", &member())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn yank_admin_bypasses_namespace() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        assert!(s
            .yank("reg", "frontend/utils", "1.0.0", &admin_id())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn yank_unclaimed_package_allows_any_user() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "unclaimed/pkg", "1.0.0"));
        let ns = MockTeamNamespace::arc(); // no claims
        let s = svc_with_ns(backend, ns);
        assert!(s
            .yank("reg", "unclaimed/pkg", "1.0.0", &non_member())
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn unyank_blocks_non_member_in_claimed_namespace() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        let err = s
            .unyank("reg", "frontend/utils", "1.0.0", &non_member())
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn unyank_allows_namespace_member() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
        let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
        let s = svc_with_ns(backend, ns);
        assert!(s
            .unyank("reg", "frontend/utils", "1.0.0", &member())
            .await
            .is_ok());
    }
}
