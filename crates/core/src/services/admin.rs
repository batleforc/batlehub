use std::sync::Arc;

use chrono::Utc;

use crate::entities::{
    AccessEvent, AccessResult, EventFilter, Identity, PackageFilter, PackageId, PackageStatus,
    PackageSummary,
};
use crate::error::CoreError;
use crate::ports::PackageRepository;

pub struct AdminService {
    pub repo: Arc<dyn PackageRepository>,
}

impl AdminService {
    pub fn new(repo: Arc<dyn PackageRepository>) -> Self {
        Self { repo }
    }

    pub async fn block_package(
        &self,
        pkg: &PackageId,
        reason: String,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        let blocked_by = by_identity
            .user_id
            .clone()
            .unwrap_or_else(|| by_identity.role.to_string());

        self.repo
            .set_status(
                pkg,
                PackageStatus::Blocked {
                    reason: reason.clone(),
                    blocked_by: blocked_by.clone(),
                    blocked_at: Utc::now(),
                },
            )
            .await?;

        self.repo
            .record_access(AccessEvent {
                id: uuid::Uuid::new_v4(),
                user_id: by_identity.user_id.clone(),
                user_role: by_identity.role.clone(),
                package_id: pkg.clone(),
                action: crate::entities::AccessAction::Block,
                result: AccessResult::Allowed,
                timestamp: Utc::now(),
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record block action"));

        tracing::info!(package = %pkg, blocked_by = %blocked_by, reason = %reason, "package blocked");
        Ok(())
    }

    pub async fn unblock_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        self.repo.set_status(pkg, PackageStatus::Available).await?;

        self.repo
            .record_access(AccessEvent {
                id: uuid::Uuid::new_v4(),
                user_id: by_identity.user_id.clone(),
                user_role: by_identity.role.clone(),
                package_id: pkg.clone(),
                action: crate::entities::AccessAction::Unblock,
                result: AccessResult::Allowed,
                timestamp: Utc::now(),
            })
            .await
            .unwrap_or_else(|e| tracing::warn!(error = %e, "failed to record unblock action"));

        tracing::info!(package = %pkg, "package unblocked");
        Ok(())
    }

    pub async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
        self.repo.list_packages(filter).await
    }

    pub async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        self.repo.list_events(filter).await
    }

    pub async fn get_package_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        self.repo.get_status(pkg).await
    }
}
