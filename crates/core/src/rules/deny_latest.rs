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
                reason:
                    "requests for the 'latest' version tag are not allowed; pin an explicit version"
                        .to_owned(),
            };
        }

        // The least-privileged bypass role is the bar, and the caller is right
        // here in `ctx` — so the comparison happens now rather than being handed
        // back for someone else to remember to make (RFC 0015 §5.1).
        let minimum = self.bypass_roles.iter().min().expect("non-empty");
        if ctx.identity.has_role_at_least(minimum) {
            RuleDecision::Allow
        } else {
            RuleDecision::Deny {
                reason: format!(
                    "requests for the 'latest' version tag are not allowed; pin an explicit \
                     version (bypass requires role '{minimum}' or higher, you have '{}')",
                    ctx.identity.role
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Action;
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
            cache_control: None,
        }
    }

    fn identity(role: Role) -> Identity {
        Identity {
            user_id: None,
            role,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn ctx<'a>(
        meta: &'a PackageMetadata,
        identity: &'a Identity,
        requested: &'a str,
    ) -> RuleContext<'a> {
        RuleContext {
            identity,
            package: meta,
            action: Action::ReleasesRead,
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

    /// A caller who lacks the bypass role is denied **by the rule**, not handed
    /// back a `RequireRole` for someone else to resolve.
    ///
    /// That indirection was the point of RFC 0015 §5.1's deletion: the identity
    /// is already in `ctx`, so deferring the comparison bought nothing and cost
    /// a class of bug where a caller forgot `.resolve()` and read the verdict as
    /// allow. Two such sites were found in `authz.rs`.
    #[tokio::test]
    async fn a_caller_without_the_bypass_role_is_denied_by_the_rule() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await;
        assert!(
            matches!(decision, RuleDecision::Deny { .. }),
            "{decision:?}"
        );
    }

    #[tokio::test]
    async fn bypass_role_allows_admin() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Admin);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await;
        assert!(matches!(decision, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn bypass_role_denies_anonymous() {
        let rule = DenyLatestRule::new(vec![Role::Admin]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &id, "latest")).await;
        assert!(matches!(decision, RuleDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn least_privileged_bypass_role_wins() {
        let rule = DenyLatestRule::new(vec![Role::Admin, Role::User]);
        let m = meta();
        let user_id = identity(Role::User);
        let decision = rule.evaluate(&ctx(&m, &user_id, "latest")).await;
        assert!(
            matches!(decision, RuleDecision::Allow),
            "User should bypass when User is in bypass_roles"
        );
    }

    #[tokio::test]
    async fn none_requested_version_allows() {
        let rule = DenyLatestRule::new(vec![]);
        let m = meta();
        let id = identity(Role::Anonymous);
        let ctx = RuleContext {
            identity: &id,
            package: &m,
            action: Action::ReleasesRead,
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }
}
