use async_trait::async_trait;

use crate::entities::Role;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Rejects requests that use the `"latest"` pseudo-version tag, encouraging
/// consumers to pin explicit versions for supply-chain hygiene.
///
/// Roles listed in `bypass_roles` may still use `"latest"`.
pub struct DenyLatestRule {
    pub bypass_roles: Vec<Role>,
}

impl DenyLatestRule {
    pub fn new(bypass_roles: Vec<Role>) -> Self {
        Self { bypass_roles }
    }
}

#[async_trait]
impl Rule for DenyLatestRule {
    fn name(&self) -> &str {
        "deny_latest"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        if ctx.requested_version != Some("latest") {
            return RuleDecision::Allow;
        }

        if self.bypass_roles.is_empty() {
            return RuleDecision::Deny {
                reason: "requests for the 'latest' version tag are not allowed; pin an explicit version".to_owned(),
            };
        }

        // Use the least-privileged bypass role as the minimum required.
        let minimum = self.bypass_roles.iter().min().expect("non-empty");
        RuleDecision::RequireRole { minimum: minimum.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{Identity, PackageId, PackageMetadata, Role};
    use crate::rules::RuleContext;
    use chrono::Utc;

    fn meta() -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new("npm", "lodash", "4.17.21"),
            published_at: Some(Utc::now()),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
        }
    }

    fn identity(role: Role) -> Identity {
        Identity { user_id: None, role, auth_provider: None, groups: vec![] }
    }

    fn ctx<'a>(meta: &'a PackageMetadata, identity: &'a Identity, requested: &'a str) -> RuleContext<'a> {
        RuleContext {
            identity,
            package: meta,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: Some(requested),
        }
    }

    #[tokio::test]
    async fn allows_pinned_version() {
        let rule = DenyLatestRule::new(vec![]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "4.17.21")).await;
        assert!(matches!(decision, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn denies_latest_with_no_bypass() {
        let rule = DenyLatestRule::new(vec![]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await;
        assert!(matches!(decision, RuleDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn requires_role_when_bypass_configured() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await;
        assert!(matches!(decision, RuleDecision::RequireRole { minimum: Role::Admin }));
    }

    #[tokio::test]
    async fn bypass_role_resolves_to_allow_for_admin() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Admin);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await.resolve(&id);
        assert!(matches!(decision, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn bypass_role_resolves_to_deny_for_anonymous() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await.resolve(&id);
        assert!(matches!(decision, RuleDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn least_privileged_bypass_role_wins() {
        let rule = DenyLatestRule::new(vec![Role::Admin, Role::User]);
        let m = meta();
        let user_id = identity(Role::User);
        let decision = rule.evaluate(&ctx(&m, &user_id, "latest")).await.resolve(&user_id);
        assert!(matches!(decision, RuleDecision::Allow), "User should bypass when User is in bypass_roles");
    }

    #[tokio::test]
    async fn none_requested_version_allows() {
        let rule = DenyLatestRule::new(vec![]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let ctx = RuleContext {
            identity: &id,
            package: &m,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }
}
