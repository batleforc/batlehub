use super::{
    AccessAction, AccessEvent, AccessResult, CoreError, Identity, LocalRegistryService, PackageId,
    PublishRequest, Role, SbomFormat, SbomPublishOptions,
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

    /// Record a successful lifecycle admin action (yank/unyank/deprecate/
    /// undeprecate/unlist/relist) through `access_log`, when configured. Mirrors
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
        let Some(repo) = self.access_log.as_ref() else {
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
