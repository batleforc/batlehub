use std::sync::Arc;

use chrono::Utc;

use crate::entities::{
    AccessEvent, AccessResult, EventFilter, ExploreEntry, ExploreFilter, Identity, PackageFilter,
    PackageId, PackageStatus, PackageSummary, RegistryStat,
};
use crate::error::CoreError;
use crate::ports::PackageRepository;

pub struct BulkBlockItem {
    pub package_id: PackageId,
    pub reason: String,
}

pub struct BulkActionResult {
    pub succeeded: Vec<PackageId>,
    pub failed: Vec<(PackageId, String)>,
}

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

    pub async fn bulk_block_packages(
        &self,
        items: Vec<BulkBlockItem>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        let mut result = BulkActionResult {
            succeeded: vec![],
            failed: vec![],
        };
        for item in items {
            match self
                .block_package(&item.package_id, item.reason, by_identity)
                .await
            {
                Ok(()) => result.succeeded.push(item.package_id),
                Err(e) => result.failed.push((item.package_id, e.to_string())),
            }
        }
        result
    }

    pub async fn bulk_unblock_packages(
        &self,
        items: Vec<PackageId>,
        by_identity: &Identity,
    ) -> BulkActionResult {
        let mut result = BulkActionResult {
            succeeded: vec![],
            failed: vec![],
        };
        for pkg in items {
            match self.unblock_package(&pkg, by_identity).await {
                Ok(()) => result.succeeded.push(pkg),
                Err(e) => result.failed.push((pkg, e.to_string())),
            }
        }
        result
    }

    pub async fn list_packages(
        &self,
        filter: PackageFilter,
    ) -> Result<Vec<PackageSummary>, CoreError> {
        self.repo.list_packages(filter).await
    }

    pub async fn count_packages(&self, filter: PackageFilter) -> Result<u64, CoreError> {
        self.repo.count_packages(filter).await
    }

    pub async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        self.repo.list_events(filter).await
    }

    pub async fn get_package_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        self.repo.get_status(pkg).await
    }

    pub async fn explore_packages(
        &self,
        filter: ExploreFilter,
    ) -> Result<Vec<ExploreEntry>, CoreError> {
        self.repo.explore_packages(filter).await
    }

    pub async fn count_explore_packages(&self, filter: ExploreFilter) -> Result<u64, CoreError> {
        self.repo.count_explore_packages(filter).await
    }

    pub async fn registry_explore_stats(
        &self,
        accessible_registries: &[String],
    ) -> Result<Vec<RegistryStat>, CoreError> {
        self.repo.registry_explore_stats(accessible_registries).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::entities::{
        AccessEvent, AccessResult, EventFilter, Identity, PackageFilter, PackageId, PackageStatus,
        PackageSummary, Role,
    };
    use crate::error::CoreError;
    use crate::ports::PackageRepository;

    struct MemRepo {
        statuses: Mutex<HashMap<String, PackageStatus>>,
        events: Mutex<Vec<AccessEvent>>,
    }

    impl MemRepo {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(HashMap::new()),
                events: Mutex::new(vec![]),
            })
        }

        fn events(&self) -> Vec<AccessEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PackageRepository for MemRepo {
        async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
        async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
            Ok(self
                .statuses
                .lock()
                .unwrap()
                .get(&pkg.cache_key())
                .cloned()
                .unwrap_or(PackageStatus::Available))
        }
        async fn set_status(
            &self,
            pkg: &PackageId,
            status: PackageStatus,
        ) -> Result<(), CoreError> {
            self.statuses
                .lock()
                .unwrap()
                .insert(pkg.cache_key(), status);
            Ok(())
        }
        async fn list_packages(&self, _f: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _f: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _f: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(self.events.lock().unwrap().clone())
        }
    }

    fn admin_identity(user_id: &str) -> Identity {
        Identity {
            user_id: Some(user_id.to_owned()),
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn anon_identity() -> Identity {
        Identity::anonymous()
    }

    #[tokio::test]
    async fn block_package_sets_blocked_status() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(
            &pkg,
            "supply chain risk".to_owned(),
            &admin_identity("alice"),
        )
        .await
        .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        assert!(status.is_blocked());
    }

    #[tokio::test]
    async fn block_package_uses_user_id_as_blocked_by() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &admin_identity("alice"))
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        match status {
            PackageStatus::Blocked { blocked_by, .. } => assert_eq!(blocked_by, "alice"),
            PackageStatus::Available => panic!("expected Blocked"),
        }
    }

    #[tokio::test]
    async fn block_package_falls_back_to_role_when_no_user_id() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &anon_identity())
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        match status {
            PackageStatus::Blocked { blocked_by, .. } => assert_eq!(blocked_by, "anonymous"),
            PackageStatus::Available => panic!("expected Blocked"),
        }
    }

    #[tokio::test]
    async fn block_package_records_audit_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "reason".to_owned(), &admin_identity("alice"))
            .await
            .unwrap();

        let events = repo.events();
        assert_eq!(events.len(), 1, "one event expected");
        assert!(matches!(events[0].result, AccessResult::Allowed));
    }

    #[tokio::test]
    async fn unblock_package_sets_available_status() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        // Block first, then unblock
        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();
        svc.unblock_package(&pkg, &admin_identity("a"))
            .await
            .unwrap();

        let status = repo.get_status(&pkg).await.unwrap();
        assert!(!status.is_blocked());
    }

    #[tokio::test]
    async fn unblock_package_records_audit_event() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "evil", "1.0.0");

        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();
        svc.unblock_package(&pkg, &admin_identity("a"))
            .await
            .unwrap();

        let events = repo.events();
        assert_eq!(events.len(), 2, "block + unblock events expected");
    }

    #[tokio::test]
    async fn get_package_status_delegates_to_repo() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "serde", "1.0.0");

        // Fresh package defaults to Available
        let status = svc.get_package_status(&pkg).await.unwrap();
        assert!(!status.is_blocked());
    }

    #[tokio::test]
    async fn list_packages_returns_repo_results() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());

        let result = svc.list_packages(PackageFilter::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn list_events_returns_repo_results() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg = PackageId::new("npm", "pkg", "1.0.0");

        svc.block_package(&pkg, "r".to_owned(), &admin_identity("a"))
            .await
            .unwrap();

        let events = svc.list_events(EventFilter::new()).await.unwrap();
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn bulk_block_succeeds_for_all_valid_packages() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let items = vec![
            BulkBlockItem {
                package_id: PackageId::new("npm", "a", "1.0.0"),
                reason: "r1".into(),
            },
            BulkBlockItem {
                package_id: PackageId::new("npm", "b", "2.0.0"),
                reason: "r2".into(),
            },
        ];
        let result = svc
            .bulk_block_packages(items, &admin_identity("alice"))
            .await;
        assert_eq!(result.succeeded.len(), 2);
        assert_eq!(result.failed.len(), 0);
        assert!(repo
            .get_status(&PackageId::new("npm", "a", "1.0.0"))
            .await
            .unwrap()
            .is_blocked());
        assert!(repo
            .get_status(&PackageId::new("npm", "b", "2.0.0"))
            .await
            .unwrap()
            .is_blocked());
    }

    #[tokio::test]
    async fn bulk_unblock_succeeds_for_all_packages() {
        let repo = MemRepo::new();
        let svc = AdminService::new(repo.clone());
        let pkg_a = PackageId::new("npm", "a", "1.0.0");
        let pkg_b = PackageId::new("npm", "b", "2.0.0");
        svc.block_package(&pkg_a, "r".into(), &admin_identity("alice"))
            .await
            .unwrap();
        svc.block_package(&pkg_b, "r".into(), &admin_identity("alice"))
            .await
            .unwrap();

        let result = svc
            .bulk_unblock_packages(vec![pkg_a.clone(), pkg_b.clone()], &admin_identity("alice"))
            .await;
        assert_eq!(result.succeeded.len(), 2);
        assert_eq!(result.failed.len(), 0);
        assert!(!repo.get_status(&pkg_a).await.unwrap().is_blocked());
        assert!(!repo.get_status(&pkg_b).await.unwrap().is_blocked());
    }
}
