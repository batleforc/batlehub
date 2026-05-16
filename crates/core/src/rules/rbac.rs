use std::collections::HashMap;

use async_trait::async_trait;

use crate::entities::Role;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Checks whether the caller's role is permitted to perform the requested operation.
///
/// The `permissions` map mirrors the `[registries.rbac]` TOML section:
/// ```toml
/// anonymous = ["releases:read"]
/// user      = ["releases:read", "source:read"]
/// admin     = ["*"]
/// ```
pub struct RbacRule {
    pub permissions: HashMap<Role, Vec<String>>,
}

impl RbacRule {
    pub fn new(permissions: HashMap<Role, Vec<String>>) -> Self {
        Self { permissions }
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
}

#[async_trait]
impl Rule for RbacRule {
    fn name(&self) -> &str {
        "rbac"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        if self.is_permitted(&ctx.identity.role, ctx.resource_type) {
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
        }
    }

    fn make_rule() -> RbacRule {
        RbacRule::new(HashMap::from([
            (Role::Anonymous, vec!["releases:read".to_owned()]),
            (Role::User, vec!["releases:read".to_owned(), "source:read".to_owned()]),
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
        };
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "releases:read", cache_entry: None };
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
        };
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "source:read", cache_entry: None };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Deny { .. }));
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
        };
        let ctx = RuleContext { identity: &identity, package: &meta, resource_type: "actions:read", cache_entry: None };
        assert!(matches!(rule.evaluate(&ctx).await, RuleDecision::Allow));
    }
}
