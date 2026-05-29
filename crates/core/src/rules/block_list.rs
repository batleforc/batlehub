use std::sync::Arc;

use async_trait::async_trait;

use crate::entities::PackageStatus;
use crate::ports::PackageRepository;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Denies access if an administrator has manually blocked the package.
pub struct BlockListRule {
    pub repo: Arc<dyn PackageRepository>,
}

impl BlockListRule {
    pub fn new(repo: Arc<dyn PackageRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl Rule for BlockListRule {
    fn name(&self) -> &str {
        "block_list"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        match self.repo.get_status(&ctx.package.id).await {
            Ok(PackageStatus::Blocked { reason, .. }) => RuleDecision::Deny { reason },
            Ok(PackageStatus::Available) => RuleDecision::Allow,
            Err(e) => {
                // SECURITY: fail-open by design — prefer availability over blocking.
                // If the DB is unreachable we allow the request through rather than
                // turning the proxy into a brick wall. Accept this trade-off only for
                // self-hosted deployments where uptime matters more than hard blocks.
                tracing::warn!(
                    package = %ctx.package.id,
                    error = %e,
                    "BlockListRule: failed to query package status, failing open"
                );
                RuleDecision::Allow
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::entities::{
        AccessEvent, EventFilter, Identity, PackageFilter, PackageId, PackageMetadata,
        PackageStatus, PackageSummary,
    };
    use crate::error::CoreError;
    use crate::ports::PackageRepository;
    use crate::rules::{Rule, RuleContext};

    struct MemRepo {
        statuses: Mutex<HashMap<String, PackageStatus>>,
    }

    impl MemRepo {
        fn available() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(HashMap::new()),
            })
        }

        fn blocked(reason: &str) -> Arc<Self> {
            let r = Arc::new(Self {
                statuses: Mutex::new(HashMap::new()),
            });
            r.statuses.lock().unwrap().insert(
                "npm/evil/1.0.0".to_owned(),
                PackageStatus::Blocked {
                    reason: reason.to_owned(),
                    blocked_by: "admin".to_owned(),
                    blocked_at: Utc::now(),
                },
            );
            r
        }
    }

    #[async_trait]
    impl PackageRepository for MemRepo {
        async fn record_access(&self, _e: AccessEvent) -> Result<(), CoreError> {
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
            Ok(vec![])
        }
    }

    struct AlwaysErrorRepo;

    #[async_trait]
    impl PackageRepository for AlwaysErrorRepo {
        async fn record_access(&self, _e: AccessEvent) -> Result<(), CoreError> {
            Ok(())
        }
        async fn get_status(&self, _pkg: &PackageId) -> Result<PackageStatus, CoreError> {
            Err(CoreError::Database("connection refused".into()))
        }
        async fn set_status(
            &self,
            _pkg: &PackageId,
            _status: PackageStatus,
        ) -> Result<(), CoreError> {
            Ok(())
        }
        async fn list_packages(&self, _f: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
            Ok(vec![])
        }
        async fn count_packages(&self, _f: PackageFilter) -> Result<u64, CoreError> {
            Ok(0)
        }
        async fn list_events(&self, _f: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
            Ok(vec![])
        }
    }

    fn make_ctx<'a>(pkg: &'a PackageMetadata, identity: &'a Identity) -> RuleContext<'a> {
        RuleContext {
            identity,
            package: pkg,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: None,
        }
    }

    fn meta(pkg_id: PackageId) -> PackageMetadata {
        PackageMetadata {
            id: pkg_id,
            published_at: Some(Utc::now() - chrono::Duration::days(10)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({}),
            cache_control: None,
        }
    }

    #[tokio::test]
    async fn available_package_returns_allow() {
        let rule = BlockListRule::new(MemRepo::available());
        let pkg_id = PackageId::new("npm", "lodash", "4.0.0");
        let m = meta(pkg_id);
        let identity = Identity::anonymous();
        let decision = rule.evaluate(&make_ctx(&m, &identity)).await;
        assert!(matches!(decision, crate::rules::RuleDecision::Allow));
    }

    #[tokio::test]
    async fn blocked_package_returns_deny_with_reason() {
        let repo = MemRepo::blocked("security vulnerability");
        let rule = BlockListRule::new(repo);
        let pkg_id = PackageId::new("npm", "evil", "1.0.0");
        let m = meta(pkg_id);
        let identity = Identity::anonymous();
        let decision = rule.evaluate(&make_ctx(&m, &identity)).await;
        assert!(
            matches!(&decision, crate::rules::RuleDecision::Deny { reason } if reason == "security vulnerability"),
            "expected Deny with correct reason, got {:?}",
            decision,
        );
    }

    #[tokio::test]
    async fn db_error_fails_open() {
        // SECURITY: intentional fail-open — DB unavailable must not block all traffic.
        let rule = BlockListRule::new(Arc::new(AlwaysErrorRepo));
        let pkg_id = PackageId::new("npm", "pkg", "1.0.0");
        let m = meta(pkg_id);
        let identity = Identity::anonymous();
        let decision = rule.evaluate(&make_ctx(&m, &identity)).await;
        assert!(
            matches!(decision, crate::rules::RuleDecision::Allow),
            "DB error must fail open"
        );
    }
}
