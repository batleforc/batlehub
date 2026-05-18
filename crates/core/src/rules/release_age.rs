use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use crate::entities::Role;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Blocks access to a release that was published less than `min_age` ago,
/// unless the caller holds one of the `bypass_roles`.
///
/// Useful for preventing immediate mirroring of brand-new releases that might
/// be yanked or recalled within a short window (supply-chain hygiene).
pub struct ReleaseAgeGateRule {
    pub min_age: Duration,
    pub bypass_roles: Vec<Role>,
}

impl ReleaseAgeGateRule {
    pub fn new(min_age: Duration, bypass_roles: Vec<Role>) -> Self {
        Self { min_age, bypass_roles }
    }
}

#[async_trait]
impl Rule for ReleaseAgeGateRule {
    fn name(&self) -> &str {
        "release_age_gate"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        // If the upstream didn't provide a publication timestamp, skip this rule.
        let Some(published_at) = ctx.package.published_at else {
            return RuleDecision::Allow;
        };

        let age = (Utc::now() - published_at).to_std().unwrap_or_default();

        if age >= self.min_age {
            return RuleDecision::Allow;
        }

        // Check if the caller's role bypasses the gate.
        if self.bypass_roles.contains(&ctx.identity.role) {
            return RuleDecision::Allow;
        }

        let remaining_secs = (self.min_age - age).as_secs();
        RuleDecision::Deny {
            reason: format!(
                "release is too recent (age: {}s, minimum: {}s, {} seconds remaining in quarantine)",
                age.as_secs(),
                self.min_age.as_secs(),
                remaining_secs,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as CDuration, Utc};

    use crate::entities::{Identity, PackageId, PackageMetadata, Role};
    use crate::rules::RuleContext;

    fn make_meta(published_at: chrono::DateTime<Utc>) -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new("github", "owner/repo", "v1.0.0"),
            published_at: Some(published_at),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
        }
    }

    fn make_identity(role: Role) -> Identity {
        Identity { user_id: None, role, auth_provider: None, groups: vec![] }
    }

    #[tokio::test]
    async fn allows_old_release() {
        let rule = ReleaseAgeGateRule::new(Duration::from_secs(3600), vec![Role::Admin]);
        let meta = make_meta(Utc::now() - CDuration::hours(2));
        let identity = make_identity(Role::Anonymous);
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "releases:read", cache_entry: None, requested_version: None };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn denies_new_release_for_anonymous() {
        let rule = ReleaseAgeGateRule::new(Duration::from_secs(3600), vec![Role::Admin]);
        let meta = make_meta(Utc::now() - CDuration::minutes(5));
        let identity = make_identity(Role::Anonymous);
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "releases:read", cache_entry: None, requested_version: None };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn admin_bypasses_age_gate() {
        let rule = ReleaseAgeGateRule::new(Duration::from_secs(3600), vec![Role::Admin]);
        let meta = make_meta(Utc::now() - CDuration::minutes(5));
        let identity = make_identity(Role::Admin);
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "releases:read", cache_entry: None, requested_version: None };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }
}
