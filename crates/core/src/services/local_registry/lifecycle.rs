use super::{
    artifact_storage_key, AccessAction, AccessEvent, AccessResult, CoreError, Identity,
    LocalRegistryService, PackageId, PublishRequest, ReadmeFormat, Role, SbomFormat,
    SbomPublishOptions,
};

impl LocalRegistryService {
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
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend.yank(registry, name, version).await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Yank, identity)
            .await;
        Ok(())
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
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend.unyank(registry, name, version).await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Unyank, identity)
            .await;
        Ok(())
    }

    pub async fn deprecate(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        message: Option<&str>,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "deprecate requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend
            .deprecate(registry, name, version, message)
            .await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Deprecate, identity)
            .await;
        Ok(())
    }

    pub async fn undeprecate(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "undeprecate requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend.undeprecate(registry, name, version).await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Undeprecate, identity)
            .await;
        Ok(())
    }

    pub async fn unlist(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "unlist requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend.unlist(registry, name, version).await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Unlist, identity)
            .await;
        Ok(())
    }

    pub async fn relist(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "relist requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;
        self.backend.relist(registry, name, version).await?;
        self.record_lifecycle_action(registry, name, version, AccessAction::Relist, identity)
            .await;
        Ok(())
    }

    /// Delete a published version: drop the bytes, keep the coordinate.
    ///
    /// This is the whole of RFC 0016 §4.4 in one place. The version row is
    /// tombstoned rather than removed, so `1.4.0` can never mean two different
    /// things to two different lockfiles — and the artifact, its README, and the
    /// explore cache entry all go, because what the caller asked for is that the
    /// bytes stop being served.
    ///
    /// Returns `true` when a published version was tombstoned, `false` when there
    /// was nothing to delete. `false` is not an error: the coordinate is gone
    /// either way, which is what the caller asked for, and a bulk delete over a
    /// list that has already been partly processed should not fail on the
    /// second pass.
    ///
    /// **Order matters.** The row is tombstoned *first*. Between that and the
    /// storage delete the version is already invisible to every listing and
    /// already refuses a re-publish, so a crash in the middle leaves an orphaned
    /// blob — recoverable, and the coherence sweep collects it. The other order
    /// would leave a *live* version pointing at bytes that are gone, which every
    /// client resolves and then fails to download.
    pub async fn delete_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<bool, CoreError> {
        // Same gate as yank/unyank. Authorization proper is RFC 0015's `releases:delete`
        // and explicitly not this document's (RFC 0016 §3); until it lands, delete is
        // reachable only through the admin-gated handler, and this is the floor under it.
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "delete requires at least User role".into(),
            ));
        }
        self.check_namespace_membership(registry, name, identity)
            .await?;
        self.check_ownership_lifecycle_access(registry, name, identity)
            .await?;

        let tombstoned = self
            .backend
            .tombstone_version(registry, name, version, identity.user_id.as_deref())
            .await?;
        if !tombstoned {
            return Ok(false);
        }

        self.drop_version_bytes(registry, name, version).await;
        self.delete_readme_for_version(registry, name, version)
            .await;
        if let Some(ref cache) = self.explore_cache {
            cache.invalidate(Some(registry)).await;
        }
        self.record_lifecycle_action(registry, name, version, AccessAction::Delete, identity)
            .await;
        Ok(true)
    }

    /// Strip aged-out tombstone detail, keeping every coordinate claim
    /// (RFC 0016 §4.5).
    ///
    /// Audited as [`AccessAction::TombstoneCompact`] rather than as a delete: it
    /// is destructive to *history* and harmless to the invariant, which is a
    /// different fact about the system and one an operator reading the trail has
    /// to be able to separate from a version being deleted. Same reasoning
    /// `AuditPurge` already follows for the audit trail's own purge.
    ///
    /// A dry run is not audited. Nothing happened, and an audit trail that
    /// records reads is a trail nobody finishes reading.
    pub async fn compact_tombstone_detail(
        &self,
        registry: &str,
        older_than: std::time::Duration,
        dry_run: bool,
        identity: &Identity,
    ) -> Result<crate::entities::CompactionReport, CoreError> {
        if !identity.has_role_at_least(&Role::Admin) {
            return Err(CoreError::AccessDenied(
                "compacting tombstone detail requires the Admin role".into(),
            ));
        }
        let report = self
            .backend
            .compact_tombstone_detail(registry, older_than, dry_run)
            .await?;
        if !dry_run && report.compacted > 0 {
            self.record_registry_action(registry, AccessAction::TombstoneCompact, identity)
                .await;
        }
        Ok(report)
    }

    /// Audit an action that is about a whole registry rather than one version.
    ///
    /// The `AccessEvent` shape wants a `PackageId`, so compaction — which
    /// touches many coordinates at once and is reported by count — records the
    /// registry with an empty name and version. That is what `AuditPurge`
    /// already does for an action with no coordinate at all, and it keeps the
    /// event joinable to a registry without inventing a package that was not
    /// involved.
    async fn record_registry_action(
        &self,
        registry: &str,
        action: AccessAction,
        identity: &Identity,
    ) {
        self.record_lifecycle_action(registry, "", "", action, identity)
            .await;
    }

    /// Drop every stored byte belonging to one version.
    ///
    /// Two calls rather than one because a version owns two shapes of key. The
    /// single-file ecosystems store the artifact *at* `local:{reg}/{name}/{ver}`;
    /// Maven and the Terraform providers store a directory of files *under* it
    /// (`…/{ver}/{filename}`, `…/{ver}/{os}-{arch}`). A prefix delete on the bare
    /// key would take neither reliably and both too much: `local:r/p/1.0` is a
    /// prefix of `local:r/p/1.0.1`, so it would delete a sibling version. The
    /// trailing slash on the prefix is what stops that.
    ///
    /// Non-fatal by construction. The tombstone is already written by the time
    /// this runs, so the version is unreachable regardless; a storage error here
    /// leaves an orphaned blob for the coherence sweep, and turning that into a
    /// failed delete would tell the caller the version is still live when it is
    /// not.
    async fn drop_version_bytes(&self, registry: &str, name: &str, version: &str) {
        let key = artifact_storage_key(registry, name, version);
        if let Err(e) = self.storage.delete(&key).await {
            tracing::warn!(
                registry, name, version, error = %e,
                "delete: dropping the artifact bytes failed; the version is tombstoned \
                 and unreachable, and the blob is left for the coherence sweep"
            );
        }
        if let Err(e) = self.storage.delete_by_prefix(&format!("{key}/")).await {
            tracing::warn!(
                registry, name, version, error = %e,
                "delete: dropping the multi-file artifacts under the version failed \
                 (non-fatal, see above)"
            );
        }
    }

    /// Record a successful lifecycle admin action (yank/unyank/deprecate/
    /// undeprecate/unlist/relist/delete) through `package_repo`, when configured. Mirrors
    /// `read.rs`'s `record_download` so these mutations aren't a silent audit
    /// gap next to the package-block/visibility/ownership admin actions that
    /// already go through `AdminService::record_admin_action`.
    pub async fn record_lifecycle_action(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        action: AccessAction,
        identity: &Identity,
    ) {
        let Some(repo) = self.package_repo.as_ref() else {
            return;
        };
        let event = AccessEvent {
            id: uuid::Uuid::new_v4(),
            user_id: identity.user_id.clone(),
            user_role: identity.role.clone(),
            package_id: Some(PackageId::new(registry, name, version)),
            action,
            result: AccessResult::Allowed,
            timestamp: chrono::Utc::now(),
            ip_address: None,
            user_agent: None,
        };
        if let Err(e) = repo.record_access(event).await {
            tracing::warn!(error = %e, "audit log write failed for local registry lifecycle action");
        }
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

    pub(super) async fn remove_pending(&self, registry: &str, name: &str, version: &str) {
        if let Err(err) = self.backend.remove_version(registry, name, version).await {
            tracing::error!("pending row cleanup failed: {err}");
        }
    }

    pub(super) async fn revoke_quota(&self, identity: &Identity, registry: &str, bytes: u64) {
        if let Some(svc) = &self.quota {
            if let Err(err) = svc.revoke_publish(identity, registry, bytes).await {
                tracing::error!("quota revoke failed: {err}");
            }
        }
    }

    /// Public revoke for path-addressed (deb/rpm) publish handlers, which record
    /// quota via [`Self::enforce_publish_policy`] and then perform their own
    /// storage writes outside the [`Self::publish`] transaction. They call this to
    /// undo the recorded quota when a write fails, so a transient storage error
    /// doesn't permanently charge the publisher for an artifact that never landed.
    pub async fn revoke_publish_quota(&self, identity: &Identity, registry: &str, bytes: u64) {
        self.revoke_quota(identity, registry, bytes).await;
    }

    /// Record a README a publish request carried in its own metadata.
    ///
    /// Called by the publish handlers that have one — npm's publish document
    /// carries `readme`, cargo's metadata carries `readme` and `readme_file` —
    /// because the field is protocol-specific and only the handler knows where
    /// to look for it. This is the shared half: the config lookup, the
    /// non-fatal contract, and the log line.
    ///
    /// Non-fatal by construction, unlike `sbom.required`: a publish that
    /// succeeded must not be reported as failed because prose could not be
    /// stored, and there is no `readme.required` for it to mean anything else.
    pub async fn record_publish_readme(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        content: String,
        format: ReadmeFormat,
    ) {
        let Some(ref readme_svc) = self.readme else {
            return;
        };
        // Absent means enabled: the builder writes an entry for every
        // configured registry, so a missing key is a hand-built test.
        let cfg = {
            let hot = self.hot.read().await;
            hot.readme.get(registry).cloned().unwrap_or_default()
        };
        if let Err(e) = readme_svc
            .record_from_publish(registry, name, version, content, format, &cfg)
            .await
        {
            tracing::warn!(
                registry, name, version, error = %e,
                "readme: recording a published README failed (non-fatal)"
            );
        }
    }

    /// Remove the stored README for a version that has just been deleted.
    ///
    /// Explicit rather than a database cascade, because `package_readmes` has no
    /// foreign key: a README outlives the bytes — the catalogue already
    /// describes versions it holds none of, and a panel that emptied itself when
    /// LRU eviction ran would be inexplicable — so a cascade from anything
    /// evictable would delete exactly the rows §5.4 says to keep. This is the
    /// other half of that decision: what nothing cascades from, something has to
    /// call.
    ///
    /// Deliberately **not** called from the admin package-delete path, which
    /// purges a *cached artifact* and its tracking row. That is eviction shaped,
    /// and the README survives it.
    pub async fn delete_readme_for_version(&self, registry: &str, name: &str, version: &str) {
        let Some(ref readme_svc) = self.readme else {
            return;
        };
        if let Err(e) = readme_svc
            .repo
            .delete_for_version(registry, name, version)
            .await
        {
            tracing::warn!(
                registry, name, version, error = %e,
                "readme: deleting a removed version's README failed (non-fatal)"
            );
        }
    }

    pub(super) async fn run_publish_sbom(
        &self,
        req: &PublishRequest,
        storage_key: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        let Some(ref sbom_svc) = self.sbom else {
            return Ok(());
        };
        let sbom_cfg = {
            let hot = self.hot.read().await;
            hot.sbom.get(&req.registry).cloned()
        };
        let Some(cfg) = sbom_cfg.filter(|c| c.enabled) else {
            return Ok(());
        };
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
                storage_key,
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
                if let Err(err) = self.storage.delete(storage_key).await {
                    tracing::error!("storage cleanup after sbom failure: {err}");
                }
                self.revoke_quota(&req.publisher, &req.registry, bytes)
                    .await;
                Err(e)
            }
            Err(e) => {
                tracing::warn!(error = %e, "sbom generation failed (non-fatal)");
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }
}
