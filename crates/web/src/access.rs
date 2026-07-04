use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use batlehub_core::entities::{Identity, Role};

/// Shared, hot-reloadable access config — updated atomically on config reload.
pub type AccessConfigLock = Arc<tokio::sync::RwLock<AccessConfig>>;

/// Convenience constructor for `AccessConfigLock`.
pub fn new_access_lock(cfg: AccessConfig) -> AccessConfigLock {
    Arc::new(tokio::sync::RwLock::new(cfg))
}

/// Maps each role (and each dynamic group) to the set of registry names it can access.
///
/// Role inheritance: user inherits anonymous, admin inherits both.
/// Groups are additive — a user sees the union of their role's registries and each group's registries.
#[derive(Clone)]
pub struct AccessConfig {
    pub anonymous: HashSet<String>,
    pub user: HashSet<String>,
    pub admin: HashSet<String>,
    /// Dynamic group → registry names. Populated from `[registries.rbac.groups]`.
    pub groups: HashMap<String, HashSet<String>>,
    /// Registries where each role can browse/search in the package explorer.
    /// Always a subset of the corresponding proxy-access set.
    pub explore_anonymous: HashSet<String>,
    pub explore_user: HashSet<String>,
    pub explore_admin: HashSet<String>,
}

impl AccessConfig {
    pub fn accessible_registries(&self, role: &Role) -> &HashSet<String> {
        match role {
            Role::Admin => &self.admin,
            Role::User => &self.user,
            Role::Anonymous => &self.anonymous,
        }
    }

    /// Returns the union of registries accessible via the caller's role and group memberships.
    /// Supports wildcard keys: `"*:team-a"` in the groups map matches `"oidc1:team-a"`,
    /// `"oidc2:team-a"`, `"kubernetes:team-a"`, etc.
    pub fn accessible_registries_for(&self, identity: &Identity) -> HashSet<String> {
        let mut result = self.accessible_registries(&identity.role).clone();
        for group in &identity.groups {
            // Exact match
            if let Some(registries) = self.groups.get(group) {
                result.extend(registries.iter().cloned());
            }
            // Wildcard match: "*:local-name" covers any provider prefix
            if let Some((_, local_name)) = group.split_once(':') {
                let wildcard = format!("*:{local_name}");
                if let Some(registries) = self.groups.get(&wildcard) {
                    result.extend(registries.iter().cloned());
                }
            }
        }
        result
    }

    pub fn has_registry_access(&self, identity: &Identity) -> bool {
        !self.accessible_registries_for(identity).is_empty()
    }

    fn explore_registries(&self, role: &Role) -> &HashSet<String> {
        match role {
            Role::Admin => &self.explore_admin,
            Role::User => &self.explore_user,
            Role::Anonymous => &self.explore_anonymous,
        }
    }

    /// Returns the set of registries the caller can browse/search in the package explorer.
    /// Groups inherit their proxy access for explore (no separate group-level explore restriction).
    pub fn explore_accessible_registries_for(&self, identity: &Identity) -> HashSet<String> {
        let proxy = self.accessible_registries_for(identity);
        let explore = self.explore_registries(&identity.role);
        proxy.intersection(explore).cloned().collect()
    }
}

#[cfg(test)]
mod access_config_tests {
    use super::*;

    fn make_config() -> AccessConfig {
        let regs: HashSet<String> = [
            "public",
            "user-only",
            "admin-only",
            "group-a-reg",
            "group-b-reg",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        AccessConfig {
            anonymous: ["public"].iter().map(|s| s.to_string()).collect(),
            user: ["public", "user-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            admin: ["public", "user-only", "admin-only"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            groups: [
                (
                    "team-a".to_owned(),
                    ["group-a-reg"].iter().map(|s| s.to_string()).collect(),
                ),
                (
                    "team-b".to_owned(),
                    ["group-b-reg", "public"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                ),
            ]
            .into_iter()
            .collect(),
            explore_anonymous: regs.clone(),
            explore_user: regs.clone(),
            explore_admin: regs,
        }
    }

    fn identity(role: Role, groups: Vec<&str>) -> Identity {
        Identity {
            user_id: None,
            role,
            auth_provider: None,
            groups: groups.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn role_only_access_unchanged() {
        let cfg = make_config();
        let id = identity(Role::User, vec![]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("public"));
        assert!(accessible.contains("user-only"));
        assert!(!accessible.contains("admin-only"));
        assert!(!accessible.contains("group-a-reg"));
    }

    #[test]
    fn group_membership_adds_group_registries() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-a"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(
            accessible.contains("group-a-reg"),
            "team-a should see group-a-reg"
        );
        assert!(
            accessible.contains("public"),
            "anonymous role still applies"
        );
        assert!(
            !accessible.contains("group-b-reg"),
            "team-a should not see group-b-reg"
        );
    }

    #[test]
    fn multiple_groups_union() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-a", "team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("group-a-reg"));
        assert!(accessible.contains("group-b-reg"));
        assert!(accessible.contains("public"));
    }

    #[test]
    fn has_registry_access_via_group_only() {
        // No role-based registries for anonymous, but group-a-reg is accessible via team-a.
        let anon_cfg = AccessConfig {
            anonymous: [].iter().cloned().collect(),
            user: [].iter().cloned().collect(),
            admin: [].iter().cloned().collect(),
            groups: [(
                "team-a".to_owned(),
                ["group-a-reg".to_string()].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            explore_anonymous: HashSet::new(),
            explore_user: HashSet::new(),
            explore_admin: HashSet::new(),
        };
        let id = identity(Role::Anonymous, vec!["team-a"]);
        assert!(anon_cfg.has_registry_access(&id));
    }

    #[test]
    fn has_registry_access_false_without_role_or_group_match() {
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-c"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("public"));
        assert!(!accessible.contains("group-a-reg"));
        assert!(!accessible.contains("group-b-reg"));
    }

    #[test]
    fn group_overlap_with_role_no_duplicates() {
        // team-b includes "public", which anonymous role already grants — no issue.
        let cfg = make_config();
        let id = identity(Role::Anonymous, vec!["team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert_eq!(accessible.iter().filter(|r| *r == "public").count(), 1);
        assert!(accessible.contains("group-b-reg"));
    }

    fn make_wildcard_config() -> AccessConfig {
        let all: HashSet<String> = ["all-reg", "shared-reg", "oidc2-reg"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        AccessConfig {
            anonymous: HashSet::new(),
            user: HashSet::new(),
            admin: ["all-reg".to_owned()].into_iter().collect(),
            groups: [
                // Wildcard: any provider's "team-a" gets "shared-reg"
                (
                    "*:team-a".to_owned(),
                    ["shared-reg".to_owned()].into_iter().collect(),
                ),
                // Exact: only oidc2's "team-b" gets "oidc2-reg"
                (
                    "oidc2:team-b".to_owned(),
                    ["oidc2-reg".to_owned()].into_iter().collect(),
                ),
            ]
            .into_iter()
            .collect(),
            explore_anonymous: all.clone(),
            explore_user: all.clone(),
            explore_admin: all,
        }
    }

    #[test]
    fn wildcard_matches_any_provider_prefix() {
        let cfg = make_wildcard_config();
        for group in &["oidc1:team-a", "oidc2:team-a", "kubernetes:team-a"] {
            let id = identity(Role::Anonymous, vec![group]);
            let accessible = cfg.accessible_registries_for(&id);
            assert!(
                accessible.contains("shared-reg"),
                "{group} should match *:team-a and access shared-reg"
            );
        }
    }

    #[test]
    fn exact_entry_not_matched_by_wrong_provider() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc1:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(
            !accessible.contains("oidc2-reg"),
            "oidc1:team-b should not match oidc2:team-b"
        );
    }

    #[test]
    fn exact_entry_matched_by_correct_provider() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc2:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(accessible.contains("oidc2-reg"));
    }

    #[test]
    fn group_without_colon_skips_wildcard_lookup_safely() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["raw-group"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(
            !accessible.contains("shared-reg"),
            "bare group name should not match wildcards"
        );
    }

    #[test]
    fn multi_provider_user_gets_union_via_wildcard() {
        let cfg = make_wildcard_config();
        let id = identity(Role::Anonymous, vec!["oidc1:team-a", "oidc2:team-b"]);
        let accessible = cfg.accessible_registries_for(&id);
        assert!(
            accessible.contains("shared-reg"),
            "wildcard match for oidc1:team-a"
        );
        assert!(
            accessible.contains("oidc2-reg"),
            "exact match for oidc2:team-b"
        );
    }
}
