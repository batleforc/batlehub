use async_trait::async_trait;

use crate::entities::Role;
use crate::rules::{Rule, RuleContext, RuleDecision};

/// Derive a normalized "publisher" identifier for a package from data already
/// resolved onto `PackageMetadata` — no extra network calls.
///
/// - `github` / `gitlab` / `forgejo`: the first `/`-separated segment of the
///   package name (`"owner/repo"` or `"group/subgroup/project"`) — present for
///   every artifact type these adapters serve, including the raw/tarball/zipball
///   paths that otherwise resolve minimal metadata.
/// - `npm`: the scope (`"@scope/name"` → `"scope"`) when scoped; otherwise the
///   publishing user recorded in `extra.publisher`.
/// - `openvsx` / `vscode-marketplace`: the first `.`-separated segment of the
///   extension id (`"{publisher}.{extension}"`).
/// - anything else (including `cargo`, where ownership isn't in the sparse
///   index and would need a separate crates.io API call): `None`.
fn derive_publisher(registry: &str, name: &str, extra: &serde_json::Value) -> Option<String> {
    match registry {
        "github" | "gitlab" | "forgejo" => name.split('/').next().map(str::to_owned),
        "npm" => {
            if let Some(scope) = name.strip_prefix('@').and_then(|s| s.split('/').next()) {
                Some(scope.to_owned())
            } else {
                extra.get("publisher")?.as_str().map(str::to_owned)
            }
        }
        "openvsx" | "vscode-marketplace" => name.split('.').next().map(str::to_owned),
        _ => None,
    }
}

/// Restricts downloads to packages published by an allowed org/user/scope.
///
/// The publisher is derived from already-resolved metadata (see
/// [`derive_publisher`]) — no extra upstream calls. A registry this rule
/// doesn't know how to derive a publisher for (including `cargo`, deferred
/// pending a crates.io owners lookup) **fails closed**: this is a supply-chain
/// trust gate, so an undeterminable publisher must not silently pass. Roles
/// listed in `bypass_roles` may still download a gated package.
///
/// Matching is case-insensitive (GitHub orgs and npm scopes are conventionally
/// case-insensitive).
///
/// ```toml
/// [[registries.rules]]
/// kind = "trusted_publisher"
/// allow = ["my-org", "trusted-user"]
/// bypass_roles = ["admin"]
/// ```
pub struct TrustedPublisherRule {
    allow: Vec<String>,
    bypass_roles: Vec<Role>,
}

impl TrustedPublisherRule {
    pub fn new(allow: &[String], bypass_roles: Vec<Role>) -> Self {
        Self {
            allow: allow.iter().map(|s| s.to_lowercase()).collect(),
            bypass_roles,
        }
    }

    /// Turn a gate violation into a `Deny`, or a `RequireRole` when bypass
    /// roles are configured (mirrors `VersionGateRule::gate`).
    fn gate(&self, reason: String) -> RuleDecision {
        if self.bypass_roles.is_empty() {
            return RuleDecision::Deny { reason };
        }
        let minimum = self.bypass_roles.iter().min().expect("non-empty").clone();
        RuleDecision::RequireRole { minimum }
    }
}

#[async_trait]
impl Rule for TrustedPublisherRule {
    fn name(&self) -> &str {
        "trusted_publisher"
    }

    async fn evaluate(&self, ctx: &RuleContext<'_>) -> RuleDecision {
        if self.allow.is_empty() {
            return RuleDecision::Allow;
        }

        let id = &ctx.package.id;
        let Some(publisher) = derive_publisher(&id.registry, &id.name, &ctx.package.extra) else {
            return self.gate(format!(
                "cannot verify the publisher of {} on registry '{}'; trusted_publisher requires \
                 a supported registry type",
                id, id.registry
            ));
        };

        if self.allow.contains(&publisher.to_lowercase()) {
            return RuleDecision::Allow;
        }

        self.gate(format!(
            "publisher '{publisher}' is not in the trusted_publisher allowlist for {id}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{Identity, PackageId, PackageMetadata, Role};
    use crate::rules::RuleContext;

    fn meta(registry: &str, name: &str, extra: serde_json::Value) -> PackageMetadata {
        PackageMetadata {
            id: PackageId::new(registry, name, "1.0.0"),
            published_at: None,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra,
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

    fn ctx<'a>(meta: &'a PackageMetadata, identity: &'a Identity) -> RuleContext<'a> {
        RuleContext {
            identity,
            package: meta,
            resource_type: "releases:read",
            cache_entry: None,
            requested_version: Some(meta.id.version.as_str()),
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn empty_allowlist_allows_everything() {
        let rule = TrustedPublisherRule::new(&[], vec![]);
        let m = meta("cargo", "serde", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn github_owner_match_allows() {
        let rule = TrustedPublisherRule::new(&strings(&["my-org"]), vec![]);
        let m = meta("github", "my-org/my-repo", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn github_owner_mismatch_denies() {
        let rule = TrustedPublisherRule::new(&strings(&["my-org"]), vec![]);
        let m = meta("github", "other-org/my-repo", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn github_match_is_case_insensitive() {
        let rule = TrustedPublisherRule::new(&strings(&["My-Org"]), vec![]);
        let m = meta("github", "my-org/my-repo", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn gitlab_nested_group_uses_top_level_group() {
        let rule = TrustedPublisherRule::new(&strings(&["group"]), vec![]);
        let m = meta("gitlab", "group/subgroup/project", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn forgejo_owner_match_allows() {
        let rule = TrustedPublisherRule::new(&strings(&["owner"]), vec![]);
        let m = meta("forgejo", "owner/repo", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn npm_scoped_package_matches_scope() {
        let rule = TrustedPublisherRule::new(&strings(&["myscope"]), vec![]);
        let m = meta("npm", "@myscope/pkg", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn npm_unscoped_package_falls_back_to_publisher_field() {
        let rule = TrustedPublisherRule::new(&strings(&["alice"]), vec![]);
        let m = meta("npm", "lodash", serde_json::json!({"publisher": "alice"}));
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn npm_unscoped_package_with_no_publisher_field_denies() {
        let rule = TrustedPublisherRule::new(&strings(&["alice"]), vec![]);
        let m = meta("npm", "lodash", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn openvsx_publisher_match_allows() {
        let rule = TrustedPublisherRule::new(&strings(&["redhat"]), vec![]);
        let m = meta("openvsx", "redhat.java", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn vscode_marketplace_publisher_match_allows() {
        let rule = TrustedPublisherRule::new(&strings(&["ms-python"]), vec![]);
        let m = meta(
            "vscode-marketplace",
            "ms-python.python",
            serde_json::Value::Null,
        );
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Allow
        ));
    }

    #[tokio::test]
    async fn unsupported_registry_fails_closed() {
        let rule = TrustedPublisherRule::new(&strings(&["anyone"]), vec![]);
        let m = meta("cargo", "serde", serde_json::Value::Null);
        let id = identity(Role::Anonymous);
        assert!(matches!(
            rule.evaluate(&ctx(&m, &id)).await,
            RuleDecision::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn bypass_role_resolves_to_allow_for_admin() {
        let rule = TrustedPublisherRule::new(&strings(&["trusted-org"]), vec![Role::Admin]);
        let m = meta("github", "other-org/repo", serde_json::Value::Null);
        let admin = identity(Role::Admin);
        let decision = rule.evaluate(&ctx(&m, &admin)).await.resolve(&admin);
        assert!(matches!(decision, RuleDecision::Allow));
    }

    #[tokio::test]
    async fn bypass_role_resolves_to_deny_for_anonymous() {
        let rule = TrustedPublisherRule::new(&strings(&["trusted-org"]), vec![Role::Admin]);
        let m = meta("github", "other-org/repo", serde_json::Value::Null);
        let anon = identity(Role::Anonymous);
        let decision = rule.evaluate(&ctx(&m, &anon)).await.resolve(&anon);
        assert!(matches!(decision, RuleDecision::Deny { .. }));
    }
}
