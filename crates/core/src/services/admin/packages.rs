use chrono::Utc;

use super::{AdminService, BulkActionResult, BulkBlockItem};
use crate::entities::{AccessAction, Identity, PackageId, PackageStatus};
use crate::error::CoreError;

impl AdminService {
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

        self.record_admin_action(Some(pkg.clone()), AccessAction::Block, by_identity)
            .await;

        tracing::info!(package = %pkg, blocked_by = %blocked_by, reason = %reason, "package blocked");
        Ok(())
    }

    pub async fn unblock_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<(), CoreError> {
        self.repo.set_status(pkg, PackageStatus::Available).await?;

        self.record_admin_action(Some(pkg.clone()), AccessAction::Unblock, by_identity)
            .await;

        tracing::info!(package = %pkg, "package unblocked");
        Ok(())
    }

    /// Record an audit event for a package-scoped admin action performed
    /// through a port other than `PackageRepository` (e.g. ownership grants,
    /// visibility changes).
    pub async fn record_package_action(
        &self,
        pkg: &PackageId,
        action: AccessAction,
        by_identity: &Identity,
    ) {
        self.record_admin_action(Some(pkg.clone()), action, by_identity)
            .await;
    }

    /// Record an audit event for an account-wide or network-wide admin
    /// action that is not scoped to any specific package (user block/unblock,
    /// IP block/unblock).
    pub async fn record_account_action(&self, action: AccessAction, by_identity: &Identity) {
        self.record_admin_action(None, action, by_identity).await;
    }

    pub async fn bulk_block_packages(
        &self,
        items: Vec<BulkBlockItem>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |item| async move {
            let outcome = self
                .block_package(&item.package_id, item.reason, by_identity)
                .await;
            (item.package_id, outcome.map_err(|e| e.to_string()))
        })
        .await
    }

    pub async fn bulk_unblock_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |pkg| async move {
            let outcome = self.unblock_package(&pkg, by_identity).await;
            (pkg, outcome.map_err(|e| e.to_string()))
        })
        .await
    }

    /// Remove a package's administrative record.
    ///
    /// Returns `true` if the row existed and was deleted. The caller is responsible
    /// for also purging the cached artifact from storage when desired.
    pub async fn delete_package(
        &self,
        pkg: &PackageId,
        by_identity: &Identity,
    ) -> Result<bool, CoreError> {
        let deleted = self.repo.delete_package(pkg).await?;
        if deleted {
            self.record_admin_action(Some(pkg.clone()), AccessAction::Delete, by_identity)
                .await;
            tracing::info!(package = %pkg, "package record deleted");
        }
        Ok(deleted)
    }

    pub async fn bulk_delete_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        self.run_bulk(items, |pkg| async move {
            let outcome = match self.delete_package(&pkg, by_identity).await {
                Ok(true) => Ok(()),
                Ok(false) => Err("package not found".to_string()),
                Err(e) => Err(e.to_string()),
            };
            (pkg, outcome)
        })
        .await
    }
}
