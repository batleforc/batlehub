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
                // Fail open: if we can't reach the DB, don't block access.
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
