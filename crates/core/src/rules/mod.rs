pub mod block_list;
pub mod cve_gate;
pub mod deny_latest;
pub mod rbac;
pub mod release_age;
pub mod version_gate;

pub use block_list::BlockListRule;
pub use cve_gate::CveGateRule;
pub use deny_latest::DenyLatestRule;
pub use rbac::RbacRule;
pub use release_age::ReleaseAgeGateRule;
pub use version_gate::VersionGateRule;

use async_trait::async_trait;

use crate::entities::{Identity, PackageMetadata, Role};
use crate::ports::CacheEntry;

pub struct RuleContext<'a> {
    /// The caller making the request.
    pub identity: &'a Identity,
    /// Resolved package metadata from the upstream registry.
    pub package: &'a PackageMetadata,
    /// The operation being requested, e.g. `"releases:read"`, `"source:read"`.
    pub resource_type: &'a str,
    /// Cached metadata entry, if one exists. Absent on the first request for a package.
    pub cache_entry: Option<&'a CacheEntry>,
    /// The version string from the original request, before upstream resolution.
    /// For example `"latest"` even if the upstream resolved it to `"1.2.3"`.
    pub requested_version: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub enum RuleDecision {
    /// The request is permitted.
    Allow,
    /// The request is rejected with a human-readable reason.
    Deny { reason: String },
    /// The current identity's role is too low; elevate to `minimum` to proceed.
    RequireRole { minimum: Role },
}

impl RuleDecision {
    pub fn is_deny(&self) -> bool {
        matches!(
            self,
            RuleDecision::Deny { .. } | RuleDecision::RequireRole { .. }
        )
    }

    /// Resolve `RequireRole` against the actual identity, returning a `Deny` if insufficient.
    pub fn resolve(self, identity: &Identity) -> Self {
        match &self {
            RuleDecision::RequireRole { minimum } => {
                if identity.has_role_at_least(minimum) {
                    RuleDecision::Allow
                } else {
                    RuleDecision::Deny {
                        reason: format!(
                            "requires role '{}' or higher (you have '{}')",
                            minimum, identity.role
                        ),
                    }
                }
            }
            other => other.clone(),
        }
    }
}

/// A single rule in the evaluation pipeline.
#[async_trait]
pub trait Rule: Send + Sync {
    /// Short identifier used in log messages (e.g. `"block_list"`, `"rbac"`).
    fn name(&self) -> &str;

    /// Evaluate the rule against `ctx`. Rules are called in order; the first
    /// `Deny` or `RequireRole` that resolves to `Deny` short-circuits the chain.
    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision;
}

/// Evaluate a list of rules in order. Returns the first `Deny`, or `Allow`.
pub async fn evaluate_rules(rules: &[Box<dyn Rule>], ctx: &RuleContext<'_>) -> RuleDecision {
    for rule in rules {
        let decision = rule.evaluate(ctx).await.resolve(ctx.identity);
        if decision.is_deny() {
            return decision;
        }
    }
    RuleDecision::Allow
}
