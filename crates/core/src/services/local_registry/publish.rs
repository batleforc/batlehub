use super::{
    artifact_storage_key, validate_package_name, validate_path_safe, validate_version, CoreError,
    Identity, LocalRegistryService, PublishRequest, PublishedPackage, QuotaCheck, Role,
    StorageMeta, Visibility,
};

/// Everything [`LocalRegistryService::enforce_publish_policy`] needs to know about
/// the artifact being published, grouped so the function takes one parameter for
/// the artifact plus `publisher` (the acting principal, kept separate since it
/// answers a different question — *who*, not *what*).
pub struct PublishPolicyRequest<'a> {
    pub registry: &'a str,
    pub name: &'a str,
    pub version: &'a str,
    pub artifact_len: u64,
    pub signature_bytes: Option<&'a [u8]>,
    pub signature_type: Option<&'a str>,
}

impl LocalRegistryService {
    /// Publish-time half of the signing policy.
    ///
    /// The two headers are independent, so a publisher can send bytes without a
    /// type — and that state used to satisfy `required` (bytes are present) and
    /// skip `allowed_types` (there is no type to check), producing a stored
    /// "signed" artifact whose signature nothing would ever verify. The download
    /// path then read it as *absent* and served the bytes unchecked, so the
    /// perverse rule was that a **bogus** type was refused and **no** type was
    /// accepted (survey finding 13).
    ///
    /// A signature is a pair. Either half alone is incoherent, and this refuses
    /// both orders of it whenever the operator has said anything about signing at
    /// all.
    ///
    /// **Zero bytes is not a signature.** `""` is valid base64, so an empty
    /// `X-Artifact-Signature` used to arrive here as `Some(&[])` — not `None`,
    /// therefore satisfying `required`, and stored as `Some(vec![])`, which the
    /// download path reads back as "this version was published signed" and
    /// `require_signed_release` then allows. Same fail-open as the pair check
    /// above, entered through the length rather than through the type, so both
    /// halves are normalised to `None` before any of the questions are asked.
    fn check_signing_policy(
        signing: &crate::services::hot_config::SigningConfig,
        sig_bytes: Option<&[u8]>,
        sig_type: Option<&str>,
    ) -> Result<(), CoreError> {
        let sig_bytes = sig_bytes.filter(|b| !b.is_empty());
        let sig_type = sig_type.filter(|t| !t.trim().is_empty());
        if signing.required && sig_bytes.is_none() {
            return Err(CoreError::AccessDenied(
                "artifact signature required (X-Artifact-Signature header missing)".into(),
            ));
        }
        match (sig_bytes, sig_type) {
            (Some(_), None) => {
                return Err(CoreError::AccessDenied(
                    "artifact signature supplied without a type (X-Signature-Type header \
                     missing); a signature that names no algorithm can never be verified"
                        .into(),
                ));
            }
            (None, Some(ty)) => {
                return Err(CoreError::AccessDenied(format!(
                    "signature type '{ty}' supplied without a signature \
                     (X-Artifact-Signature header missing)"
                )));
            }
            _ => {}
        }
        if !signing.allowed_types.is_empty() {
            // `None` is now unreachable with bytes present, but the list is still
            // consulted through a match rather than an `if let`: an absent type
            // short-circuiting this check is the exact shape of the finding, and
            // it should not be reintroducible by someone relaxing the pair check
            // above.
            match sig_type {
                Some(st) if signing.allowed_types.iter().any(|t| t == st) => {}
                Some(st) => {
                    return Err(CoreError::AccessDenied(format!(
                        "signature type '{st}' is not in the allowed list"
                    )));
                }
                // No signature at all. Whether that is acceptable is
                // `signing.required`'s question, already answered above.
                None => {}
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

    /// Same ownership gate as [`Self::check_ownership_publish_access`], for
    /// lifecycle mutations (yank/unyank/deprecate/undeprecate/unlist/relist)
    /// on an already-published package rather than a new upload. `can_publish`
    /// already returns `true` for a package with no recorded owners, so this
    /// is a no-op both when ownership tracking is disabled and for unclaimed
    /// packages — it only blocks a `User`-role identity that isn't an owner of
    /// an already-claimed package.
    pub(super) async fn check_ownership_lifecycle_access(
        &self,
        registry: &str,
        name: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        let Some(ref ownership) = self.ownership else {
            return Ok(());
        };
        if !ownership.can_publish(registry, name, identity).await? {
            return Err(CoreError::AccessDenied(format!(
                "you are not an owner of '{name}' in registry '{registry}'"
            )));
        }
        Ok(())
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
    pub async fn enforce_publish_policy(
        &self,
        req: &PublishPolicyRequest<'_>,
        publisher: &Identity,
    ) -> Result<(QuotaCheck, bool), CoreError> {
        if !publisher.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "publishing requires at least User role".into(),
            ));
        }

        // Reject names/versions that could escape the storage root via path
        // traversal once interpolated into the storage key. Runs unconditionally,
        // independent of the optional versioning policy below.
        validate_package_name(req.name)?;
        validate_path_safe("version", req.version)?;

        // A spent coordinate is refused before anything else is decided
        // (RFC 0016 §4.4). Ahead of the quota reservation on purpose: a publish
        // that can never succeed should not first charge the publisher for bytes
        // and then hand them back. The backends refuse it a second time in
        // `publish()` — that one is the invariant, this one is the clean error and
        // the one the path-addressed (deb/rpm) publishers also pass through.
        if let Some(ts) = self
            .backend
            .find_tombstone(req.registry, req.name, req.version)
            .await?
        {
            return Err(CoreError::Conflict(ts.burned_coordinate_message()));
        }

        // Snapshot hot-swappable policy (versioning, signing, size limit).
        let (versioning, signing, limit) = {
            let hot = self.hot.read().await;
            let versioning = hot.versioning.get(req.registry).cloned();
            let signing = hot.signing.get(req.registry).cloned();
            let limit = hot.max_artifact_size_bytes.unwrap_or(500 * 1024 * 1024);
            (versioning, signing, limit)
        };

        // Versioning policy check.
        if let Some(ref policy) = versioning {
            validate_version(req.version, policy)?;
        }

        // Signing check.
        if let Some(ref signing) = signing {
            Self::check_signing_policy(signing, req.signature_bytes, req.signature_type)?;
        }

        // Namespace enforcement.
        self.check_namespace_membership(req.registry, req.name, publisher)
            .await?;

        // Ownership check.
        let is_new_package = self
            .check_ownership_publish_access(req.registry, req.name, publisher)
            .await?;

        // `limit` was extracted from hot config above.
        if req.artifact_len > limit {
            return Err(CoreError::PayloadTooLarge(format!(
                "artifact is {} bytes; limit is {limit}",
                req.artifact_len
            )));
        }

        // Check and record quota before persisting. This may return QuotaExceeded.
        let quota_check = if let Some(quota_svc) = &self.quota {
            quota_svc
                .check_and_record_publish(publisher, req.registry, req.artifact_len)
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
                &PublishPolicyRequest {
                    registry: &req.registry,
                    name: &req.name,
                    version: &req.version,
                    artifact_len: req.artifact.len() as u64,
                    signature_bytes: req.signature_bytes.as_deref(),
                    signature_type: req.signature_type.as_deref(),
                },
                &req.publisher,
            )
            .await?;

        // Inherit the existing package visibility so that publishing a new version
        // doesn't silently reset a team/internal package back to public.
        // `get_visibility` returns Public when no published rows exist yet (first publish).
        // Propagate DB errors rather than defaulting to Public — silently publishing a
        // team-private package as world-readable during a DB outage is a security failure.
        let visibility = if let Some(ref ns_port) = self.team_namespace {
            // Quota was already reserved-and-recorded in `enforce_publish_policy`.
            // A failure here (before `execute_publish_transaction`, the only place
            // that revokes on rollback) would otherwise charge the publisher for
            // bytes that are never stored, so revoke the reservation before
            // propagating the error.
            match ns_port.get_visibility(&req.registry, &req.name).await {
                Ok(v) => v,
                Err(e) => {
                    self.revoke_quota(&req.publisher, &req.registry, req.artifact.len() as u64)
                        .await;
                    return Err(e);
                }
            }
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
            unlisted: req.unlisted,
            index_metadata: req.index_metadata.clone(),
            published_at: chrono::Utc::now(),
            published_by: req.publisher.user_id.clone(),
            // Normalised the same way `check_signing_policy` judged them: an
            // empty signature is no signature, and persisting `Some(vec![])`
            // would make this row read back as "published signed" for
            // `require_signed_release` while carrying nothing to verify.
            signature_bytes: req.signature_bytes.clone().filter(|b| !b.is_empty()),
            signature_type: req.signature_type.clone().filter(|t| !t.trim().is_empty()),
            visibility,
            // Never pinned on publish. A retention pin is an operator's later
            // decision about a specific release; a publisher who could set it
            // would make every version exempt from the policy above them.
            retention_keep: false,
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

        // Step 4: generate SBOM. When `required` is true and generation fails,
        // roll back the publish (version row + bytes + quota) and return the
        // error. This runs *before* owner registration so a rejected publish
        // never leaves a dangling owner claim on a name with no versions.
        self.run_publish_sbom(&req, &storage_key, bytes).await?;

        // Step 5: on first publish, register the publisher as the package admin.
        // Last, and only once the publish is fully committed, so it is never
        // orphaned by a later rollback.
        self.register_initial_owner(is_new_package, &req.registry, &req.name, &req.publisher)
            .await;

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
