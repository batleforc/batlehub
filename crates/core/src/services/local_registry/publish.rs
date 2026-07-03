use super::{
    artifact_storage_key, validate_package_name, validate_path_safe, validate_version, CoreError,
    Identity, LocalRegistryService, PublishRequest, PublishedPackage, QuotaCheck, Role,
    StorageMeta, Visibility,
};

impl LocalRegistryService {
    fn check_signing_policy(
        signing: &crate::services::hot_config::SigningConfig,
        sig_bytes: Option<&Vec<u8>>,
        sig_type: Option<&String>,
    ) -> Result<(), CoreError> {
        if signing.required && sig_bytes.is_none() {
            return Err(CoreError::AccessDenied(
                "artifact signature required (X-Artifact-Signature header missing)".into(),
            ));
        }
        if !signing.allowed_types.is_empty() {
            if let Some(st) = sig_type {
                if !signing.allowed_types.iter().any(|t| t == st) {
                    return Err(CoreError::AccessDenied(format!(
                        "signature type '{st}' is not in the allowed list"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Returns `true` when the package is new (no existing version).
    async fn check_ownership_publish_access(
        &self,
        registry: &str,
        name: &str,
        publisher: &Identity,
    ) -> Result<bool, CoreError> {
        let Some(ref ownership) = self.ownership else {
            return Ok(false);
        };
        let package_exists = self.backend.exists(registry, name).await?;
        if package_exists && !ownership.can_publish(registry, name, publisher).await? {
            return Err(CoreError::AccessDenied(format!(
                "you are not an owner of '{name}' in registry '{registry}'"
            )));
        }
        Ok(!package_exists)
    }

    /// Enforce the publish-time policy that every registry shares — role,
    /// name/version validation, versioning policy, signing policy, namespace,
    /// ownership, artifact size limit, and quota — *without* committing a
    /// package-version row or storing bytes.
    ///
    /// Path-addressed registries (deb/rpm) host their packages under a custom
    /// storage layout rather than the `{registry}/{name}/{version}` key + DB
    /// version row that [`Self::publish`] manages, so they call this directly
    /// before their own storage work to avoid bypassing the configured limits.
    ///
    /// Returns the post-publish [`QuotaCheck`] (for `X-Quota-*` headers) and
    /// whether this is the first time the package name is seen.
    #[allow(clippy::too_many_arguments)]
    pub async fn enforce_publish_policy(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        artifact_len: u64,
        publisher: &Identity,
        signature_bytes: Option<&Vec<u8>>,
        signature_type: Option<&String>,
    ) -> Result<(QuotaCheck, bool), CoreError> {
        if !publisher.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "publishing requires at least User role".into(),
            ));
        }

        // Reject names/versions that could escape the storage root via path
        // traversal once interpolated into the storage key. Runs unconditionally,
        // independent of the optional versioning policy below.
        validate_package_name(name)?;
        validate_path_safe("version", version)?;

        // Snapshot hot-swappable policy (versioning, signing, size limit).
        let (versioning, signing, limit) = {
            let hot = self.hot.read().await;
            let versioning = hot.versioning.get(registry).cloned();
            let signing = hot.signing.get(registry).cloned();
            let limit = hot.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
            (versioning, signing, limit)
        };

        // Versioning policy check.
        if let Some(ref policy) = versioning {
            validate_version(version, policy)?;
        }

        // Signing check.
        if let Some(ref signing) = signing {
            Self::check_signing_policy(signing, signature_bytes, signature_type)?;
        }

        // Namespace enforcement.
        self.check_namespace_membership(registry, name, publisher)
            .await?;

        // Ownership check.
        let is_new_package = self
            .check_ownership_publish_access(registry, name, publisher)
            .await?;

        // `limit` was extracted from hot config above.
        if artifact_len > limit {
            return Err(CoreError::PayloadTooLarge(format!(
                "artifact is {artifact_len} bytes; limit is {limit}"
            )));
        }

        // Check and record quota before persisting. This may return QuotaExceeded.
        let quota_check = if let Some(quota_svc) = &self.quota {
            quota_svc
                .check_and_record_publish(publisher, registry, artifact_len)
                .await?
        } else {
            QuotaCheck::default()
        };

        Ok((quota_check, is_new_package))
    }

    /// Validate and persist a published artifact.
    ///
    /// Returns a `QuotaCheck` describing the publisher's current quota state
    /// after the publish (useful for setting `X-Quota-*` response headers).
    /// Returns a zeroed `QuotaCheck` when no quota is configured.
    pub async fn publish(&self, req: PublishRequest) -> Result<QuotaCheck, CoreError> {
        let (quota_check, is_new_package) = self
            .enforce_publish_policy(
                &req.registry,
                &req.name,
                &req.version,
                req.artifact.len() as u64,
                &req.publisher,
                req.signature_bytes.as_ref(),
                req.signature_type.as_ref(),
            )
            .await?;

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
            deprecated: false,
            deprecation_message: None,
            unlisted: false,
            index_metadata: req.index_metadata.clone(),
            published_at: chrono::Utc::now(),
            published_by: req.publisher.user_id.clone(),
            signature_bytes: req.signature_bytes.clone(),
            signature_type: req.signature_type.clone(),
            visibility,
        };

        let storage_key = artifact_storage_key(&req.registry, &req.name, &req.version);
        let bytes = req.artifact.len() as u64;

        // Steps 1-3: reserve → store → commit, with rollback on each failure.
        self.execute_publish_transaction(pkg, &req, &storage_key, bytes)
            .await?;

        // Invalidate explore cache so the new version appears without waiting for TTL expiry.
        if let Some(ref cache) = self.explore_cache {
            cache.invalidate(Some(&req.registry)).await;
        }

        // Step 4: on first publish, register the publisher as the package admin.
        self.register_initial_owner(is_new_package, &req.registry, &req.name, &req.publisher)
            .await;

        // Step 5: generate SBOM. When `required` is true and generation fails,
        // roll back the publish and return the error.
        self.run_publish_sbom(&req, &storage_key, bytes).await?;

        Ok(quota_check)
    }

    /// Steps 1-3 of publish: reserve pending row → store artifact bytes → commit.
    /// Rolls back cleanly on each failure so the caller gets a pristine error.
    async fn execute_publish_transaction(
        &self,
        pkg: PublishedPackage,
        req: &PublishRequest,
        storage_key: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        let publisher = &req.publisher;
        let registry = req.registry.as_str();
        let name = req.name.as_str();
        let version = req.version.as_str();

        // Step 1: reserve the version (inserted as 'pending', invisible to readers).
        if let Err(e) = self.backend.publish(pkg).await {
            self.revoke_quota(publisher, registry, bytes).await;
            return Err(e);
        }

        // Step 2: persist artifact bytes. On failure, discard the pending row.
        if let Err(e) = self
            .storage
            .store(
                storage_key,
                req.artifact.clone(),
                StorageMeta {
                    content_type: Some("application/octet-stream".into()),
                    size: None,
                    checksum: Some(req.checksum.clone()),
                },
            )
            .await
        {
            self.remove_pending(registry, name, version).await;
            self.revoke_quota(publisher, registry, bytes).await;
            return Err(e);
        }

        // Step 3: promote the pending row to 'published'. On failure, undo both
        // the storage write and the pending row so the caller gets a clean error.
        if let Err(e) = self.backend.commit_publish(registry, name, version).await {
            self.remove_pending(registry, name, version).await;
            if let Err(err) = self.storage.delete(storage_key).await {
                tracing::error!("storage cleanup after commit failure: {err}");
            }
            self.revoke_quota(publisher, registry, bytes).await;
            return Err(e);
        }

        Ok(())
    }

    /// Step 4 of publish: register the publisher as package admin on first publish (non-fatal).
    async fn register_initial_owner(
        &self,
        is_new_package: bool,
        registry: &str,
        name: &str,
        publisher: &Identity,
    ) {
        if !is_new_package {
            return;
        }
        if let (Some(ref ownership), Some(ref uid)) = (&self.ownership, &publisher.user_id) {
            if let Err(err) = ownership.initialize_owner(registry, name, uid).await {
                tracing::warn!("initialize_owner failed (non-fatal): {err}");
            }
        }
    }
}
