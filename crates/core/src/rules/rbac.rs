use std::collections::HashMap;

use async_trait::async_trait;

use crate::entities::Role;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Checks whether the caller's role or group membership permits the requested operation.
///
/// The `permissions` map mirrors the `[registries.rbac]` TOML section:
/// ```toml
/// anonymous = ["releases:read"]
/// user      = ["releases:read", "source:read"]
/// admin     = ["*"]
/// [registries.rbac.groups]
/// "team-a" = ["releases:read", "source:read"]
/// ```
pub struct RbacRule {
    pub permissions: HashMap<Role, Vec<String>>,
    pub group_permissions: HashMap<String, Vec<String>>,
}

impl RbacRule {
    pub fn new(permissions: HashMap<Role, Vec<String>>) -> Self {
        Self {
            permissions,
            group_permissions: HashMap::new(),
        }
    }

    pub fn with_groups(mut self, group_permissions: HashMap<String, Vec<String>>) -> Self {
        self.group_permissions = group_permissions;
        self
    }

    fn is_permitted(&self, role: &Role, resource_type: &str) -> bool {
        // Walk from the requested role down to Anonymous, granting if any level permits.
        // This implements role inheritance: Admin inherits User's permissions, etc.
        let roles_to_check: Vec<&Role> = match role {
            Role::Admin => vec![&Role::Admin, &Role::User, &Role::Anonymous],
            Role::User => vec![&Role::User, &Role::Anonymous],
            Role::Anonymous => vec![&Role::Anonymous],
        };

        for check_role in roles_to_check {
            if let Some(perms) = self.permissions.get(check_role) {
                if perms.iter().any(|p| p == "*" || p == resource_type) {
                    return true;
                }
            }
        }
        false
    }

    fn perms_allow(&self, key: &str, resource_type: &str) -> bool {
        self.group_permissions
            .get(key)
            .map(|perms| perms.iter().any(|p| p == "*" || p == resource_type))
            .unwrap_or(false)
    }

    fn is_permitted_by_group(&self, groups: &[String], resource_type: &str) -> bool {
        groups.iter().any(|g| {
            // Exact match: "oidc1:team-a"
            if self.perms_allow(g, resource_type) {
                return true;
            }
            // Wildcard match: "*:team-a" covers any provider prefix
            if let Some(colon) = g.find(':') {
                let wildcard = format!("*:{}", &g[colon + 1..]);
                if self.perms_allow(&wildcard, resource_type) {
                    return true;
                }
            }
            false
        })
    }
}

#[async_trait]
impl Rule for RbacRule {
    fn name(&self) -> &str {
        "rbac"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        if self.is_permitted(&ctx.identity.role, ctx.resource_type)
            || self.is_permitted_by_group(&ctx.identity.groups, ctx.resource_type)
        {
            RuleDecision::Allow
        } else {
            RuleDecision::Deny {
                reason: format!(
                    "role '{}' is not permitted to perform '{}' on this registry",
                    ctx.identity.role, ctx.resource_type
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Identity;

    fn make_identity(role: Role) -> Identity {
        Identity {
            user_id: None,
            role,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn make_identity_with_groups(role: Role, groups: Vec<&str>) -> Identity {
        Identity {
            user_id: None,
            role,
            auth_provider: None,
            groups: groups.into_iter().map(str::to_owned).collect(),
        }
    }

    fn make_rule() -> RbacRule {
        RbacRule::new(HashMap::from([
            (Role::Anonymous, vec!["releases:read".to_owned()]),
            (
                Role::User,
                vec!["releases:read".to_owned(), "source:read".to_owned()],
            ),
            (Role::Admin, vec!["*".to_owned()]),
        ]))
    }

    #[tokio::test]
    async fn anonymous_can_read_releases() {
        let rule = make_rule();
        let identity = make_identity(Role::Anonymous);
        let meta = crate::entities::PackageMetadata {
            id: crate::entities::PackageId::new("github", "rust-lang/rust", "v1.80.0"),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        };
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn anonymous_cannot_read_source() {
        let rule = make_rule();
        let identity = make_identity(Role::Anonymous);
        let meta = crate::entities::PackageMetadata {
            id: crate::entities::PackageId::new("github", "rust-lang/rust", "v1.80.0"),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        };
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(
            rule.evaluate(&ctx).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn admin_can_do_anything() {
        let rule = make_rule();
        let identity = make_identity(Role::Admin);
        let meta = crate::entities::PackageMetadata {
            id: crate::entities::PackageId::new("github", "rust-lang/rust", "v1.80.0"),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        };
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "actions:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    fn make_group_rule() -> RbacRule {
        RbacRule::new(HashMap::from([
            (Role::Anonymous, vec![]),
            (Role::User, vec!["releases:read".to_owned()]),
            (Role::Admin, vec!["*".to_owned()]),
        ]))
        .with_groups(HashMap::from([
            (
                "team-a".to_owned(),
                vec!["releases:read".to_owned(), "source:read".to_owned()],
            ),
            ("team-b".to_owned(), vec!["releases:read".to_owned()]),
        ]))
    }

    fn make_meta() -> crate::entities::PackageMetadata {
        crate::entities::PackageMetadata {
            id: crate::entities::PackageId::new("github", "rust-lang/rust", "v1.80.0"),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control: None,
        }
    }

    #[tokio::test]
    async fn group_member_can_access_group_registry() {
        let rule = make_group_rule();
        let identity = make_identity_with_groups(Role::Anonymous, vec!["team-a"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn non_group_member_cannot_access_group_only_resource() {
        let rule = make_group_rule();
        let identity = make_identity_with_groups(Role::Anonymous, vec!["team-b"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(
            rule.evaluate(&ctx).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn multi_group_member_sees_union_of_permissions() {
        let rule = make_group_rule();
        let identity = make_identity_with_groups(Role::Anonymous, vec!["team-b", "team-a"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    fn make_wildcard_rule() -> RbacRule {
        RbacRule::new(HashMap::from([
            (Role::Anonymous, vec![]),
            (Role::User, vec![]),
            (Role::Admin, vec!["*".to_owned()]),
        ]))
        .with_groups(HashMap::from([
            // Wildcard entry: any provider's "team-a" group gets releases:read
            ("*:team-a".to_owned(), vec!["releases:read".to_owned()]),
            // Exact entry: only oidc2's "team-b" gets source:read
            ("oidc2:team-b".to_owned(), vec!["source:read".to_owned()]),
        ]))
    }

    #[tokio::test]
    async fn wildcard_prefix_matches_any_provider() {
        let rule = make_wildcard_rule();
        let meta = make_meta();
        for provider_group in &["oidc1:team-a", "oidc2:team-a", "kubernetes:team-a"] {
            let identity = make_identity_with_groups(Role::Anonymous, vec![provider_group]);
            let ctx = RuleContext {
                identity: &identity,
                package: &meta,
                resource_type: "releases:read",
                cache_entry: None,
                requested_version: None,
            };
            assert!(
                matches!(rule.evaluate(&ctx).await, RuleDecision::Allow),
                "{provider_group} should match wildcard *:team-a"
            );
        }
    }

    #[tokio::test]
    async fn exact_entry_does_not_match_wrong_provider() {
        let rule = make_wildcard_rule();
        // "oidc2:team-b" has source:read; "oidc1:team-b" should NOT match (no wildcard for team-b)
        let identity = make_identity_with_groups(Role::Anonymous, vec!["oidc1:team-b"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(
            rule.evaluate(&ctx).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn exact_entry_matches_correct_provider() {
        let rule = make_wildcard_rule();
        let identity = make_identity_with_groups(Role::Anonymous, vec!["oidc2:team-b"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "source:read",
            cache_entry: None,
            requested_version: None,
        };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn group_without_colon_does_not_panic_on_wildcard_lookup() {
        let rule = make_wildcard_rule();
        let identity = make_identity_with_groups(Role::Anonymous, vec!["no-prefix-group"]);
        let meta = make_meta();
        let ctx = RuleContext {
            identity: &identity,
            package: &meta,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: None,
        };
        // Should not match any entry and not panic
        assert!(matches!(
            rule.evaluate(&ctx).await,
            RuleDecision::Deny { .. }
        ));
    }
}
