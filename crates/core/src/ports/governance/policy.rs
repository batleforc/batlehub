//! Storage for the two policy tiers a config file cannot express.
//!
//! RFC 0015 §6.3: *"One `policy` table keyed `(registry, tier, node_key)`
//! carrying every policy kind for the package and version tiers — not a table
//! per feature, and not one per tier."* Same reasoning as
//! [`GrantRepository`](super::GrantRepository)'s: §4.1 notes that a registry
//! with 200 000 packages will not enumerate them in TOML, let alone their two
//! million versions, so these two tiers are written through the admin API.
//!
//! # Why one table and not one per policy
//!
//! Because the alternative multiplies with the feature list rather than with the
//! model. `visibility`, `versioning`, `quota`, `rules` and RFC 0016's
//! `retention` are five policies over two tiers, and a table per pair is ten
//! places for a resolver to look — nine of which are usually empty for any given
//! coordinate. The composition rules in
//! [`PolicyPath`](crate::entities::PolicyPath) walk a node at a time, so what
//! storage owes them is *the node*, whole, in one read.
//!
//! Concretely: [`PolicyRepository::policy_for`] answers both tiers in one call,
//! for the same reason `grants_for` does — resolution needs them together, and
//! §11.7's resolution budget is a 2 ms p99 that a second round trip spends.
//!
//! # There is no seal here either
//!
//! For the reason the grants port gives at length: sealing is confined to the
//! config file because it is the one construct that takes access away. Nothing
//! in this port takes access away either — every policy it stores is a
//! *constraint on a resource* rather than a permission, and the one that narrows
//! an audience (`visibility = "private"`) is a scalar the administrative floor
//! sits above.

use async_trait::async_trait;

use super::grants::NodeKind;
use crate::entities::{QuotaRules, RuleOverride, VersioningRules, Visibility};
use crate::error::CoreError;

/// One node's stored policy — the package- or version-tier half of §4.1.
///
/// Every field is `Option`/empty and means **inherit**, exactly as
/// [`PolicyNode`](crate::entities::PolicyNode)'s do. A stored row that set a
/// field to its default rather than leaving it absent would be an *override*
/// with a default value, which is a different statement and would stop the tier
/// above from applying.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredPolicy {
    pub registry: String,
    pub node_kind: NodeKind,
    pub node_key: String,
    pub visibility: Option<Visibility>,
    pub prerelease_visibility: Option<Visibility>,
    pub versioning: Option<VersioningRules>,
    pub quota: Option<QuotaRules>,
    pub rules: Vec<RuleOverride>,
    pub set_by: Option<String>,
}

impl StoredPolicy {
    pub fn new(
        registry: impl Into<String>,
        node_kind: NodeKind,
        node_key: impl Into<String>,
    ) -> Self {
        Self {
            registry: registry.into(),
            node_kind,
            node_key: node_key.into(),
            ..Default::default()
        }
    }

    /// Whether this row says anything at all.
    ///
    /// A row that declares nothing is not a policy, it is a row — and storing
    /// one would make "has a policy node" and "has a policy" different
    /// questions, which is precisely the `Option`-collapsing mistake §4.3 warns
    /// about in the grant model.
    pub fn is_empty(&self) -> bool {
        self.visibility.is_none()
            && self.prerelease_visibility.is_none()
            && self.versioning.is_none()
            && self.quota.is_none()
            && self.rules.is_empty()
    }

    /// §4.1 and §4.9: which fields this row may legally carry, given its tier.
    ///
    /// Returns the reason it may not, or `None` when it is fine. The rules are
    /// small and each has a stated cause rather than being a taste:
    ///
    /// - **The naming half of `versioning` is meaningless at version tier**,
    ///   where the name already exists. `enforce_semver` on `1.4.0` has nothing
    ///   left to decide, so it is rejected rather than silently ignored.
    ///   `immutable` is the exception and the reason the tier exists at all:
    ///   freezing one golden build inside a namespace that otherwise permits
    ///   replacement.
    /// - **`quota` stops at the package tier.** A per-version quota would limit
    ///   a thing published exactly once.
    pub fn validate(&self) -> Option<String> {
        if self.node_kind != NodeKind::Version {
            return None;
        }
        if self
            .versioning
            .as_ref()
            .is_some_and(|v| v.declares_naming_fields())
        {
            return Some(
                "a version-tier policy may only set `immutable`: the naming fields \
                 (enforce_semver, allow_prerelease, version_pattern, monotonic) govern what a \
                 version may be *called*, and at this tier the name already exists"
                    .to_owned(),
            );
        }
        if self.quota.is_some() {
            return Some(
                "quota stops at the package tier: a per-version quota would limit a thing that \
                 is published exactly once"
                    .to_owned(),
            );
        }
        None
    }
}

impl Default for NodeKind {
    /// Package, because it is the tier that exists for every coordinate.
    ///
    /// Only so [`StoredPolicy`] can derive `Default` for its builder; every
    /// constructor sets it explicitly.
    fn default() -> Self {
        NodeKind::Package
    }
}

#[async_trait]
pub trait PolicyRepository: Send + Sync {
    /// The stored policy on the package and version nodes for one coordinate.
    ///
    /// **One call, both tiers**, and returned deepest-last so the caller can
    /// append the rows straight onto the config-derived path without sorting —
    /// `PolicyPath::resolve` composes in order, and a caller who had to
    /// re-establish that order would be a second implementation of the tier
    /// hierarchy.
    ///
    /// `version` is optional: a listing names a package and no version.
    async fn policy_for(
        &self,
        registry: &str,
        package: &str,
        version: Option<&str>,
    ) -> Result<Vec<StoredPolicy>, CoreError>;

    /// Write one node's policy, replacing whatever was on it.
    ///
    /// Wholesale for the node, which is not the same as wholesale composition:
    /// this replaces *this node's* declaration, and what it inherits is
    /// unaffected. An empty policy is a delete — see [`StoredPolicy::is_empty`].
    async fn put_policy(&self, policy: StoredPolicy) -> Result<(), CoreError>;

    /// Remove a node's policy. Absent is not an error.
    async fn delete_policy(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<(), CoreError>;

    /// One node's policy, for the admin API and `explain`.
    async fn policy_on_node(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<Option<StoredPolicy>, CoreError>;

    /// Every version-tier row in a registry that carries a gate exemption
    /// (RFC 0015 §4.5).
    ///
    /// The Exemptions panel of §4.8's page, and the reason it is a query rather
    /// than a walk: an exemption is a **deliberate weakening**, and the page
    /// exists because *"a shadowed grant, a self-approved exemption and a
    /// retention run about to go live are each individually easy to forget, and
    /// collectively they are the list of everything currently trusting an
    /// operator to remember."* A list nobody can produce is a list nobody reads.
    ///
    /// Returns rows, not exemptions: expiry filtering belongs to the caller,
    /// which has a clock, and an expired exemption is still worth showing on a
    /// page whose subject is what has been weakened and when it lapses.
    async fn exemptions_in_registry(&self, registry: &str) -> Result<Vec<StoredPolicy>, CoreError>;

    /// Drop every policy row keyed by a package name, at both tiers.
    ///
    /// The twin of `delete_package_grants`, and RFC 0016 §4.4's rule applies
    /// identically: **package-tier policy dies with the package.** A stale
    /// `visibility = "public"` outliving a package would silently apply to
    /// whoever takes the name next, which is finding 1's stale-claim shape with
    /// a policy instead of an owner.
    async fn delete_package_policy(&self, registry: &str, package: &str) -> Result<(), CoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Immutable;

    fn version_policy() -> StoredPolicy {
        StoredPolicy::new("npm1", NodeKind::Version, "pkg@1.0.0")
    }

    /// The one `versioning` field a version tier may carry, and the reason the
    /// tier exists: one golden build frozen inside a namespace that otherwise
    /// permits replacement.
    #[test]
    fn a_version_tier_may_pin_immutability() {
        let mut p = version_policy();
        p.versioning = Some(VersioningRules {
            immutable: Immutable::Always,
            allow_prerelease: true,
            ..Default::default()
        });
        assert_eq!(p.validate(), None);
    }

    /// …and may not carry the naming fields, where the name already exists.
    #[test]
    fn a_version_tier_may_not_carry_the_naming_fields() {
        for rules in [
            VersioningRules {
                enforce_semver: true,
                allow_prerelease: true,
                ..Default::default()
            },
            VersioningRules {
                monotonic: true,
                allow_prerelease: true,
                ..Default::default()
            },
            VersioningRules {
                version_pattern: Some("^1".to_owned()),
                allow_prerelease: true,
                ..Default::default()
            },
            // `allow_prerelease = false` is a naming field too: it governs what
            // may be published, and at version tier it already was.
            VersioningRules::default(),
        ] {
            let mut p = version_policy();
            p.versioning = Some(rules);
            assert!(
                p.validate().is_some_and(|e| e.contains("already exists")),
                "{p:?}"
            );
        }
    }

    /// A per-version quota would limit a thing published exactly once.
    #[test]
    fn a_version_tier_may_not_carry_a_quota() {
        let mut p = version_policy();
        p.quota = Some(QuotaRules::default());
        assert!(p
            .validate()
            .is_some_and(|e| e.contains("published exactly once")));
    }

    /// The package tier carries everything, which is what makes the rejections
    /// above about the *version* tier rather than about the fields.
    #[test]
    fn a_package_tier_carries_everything() {
        let mut p = StoredPolicy::new("npm1", NodeKind::Package, "pkg");
        p.versioning = Some(VersioningRules {
            enforce_semver: true,
            monotonic: true,
            allow_prerelease: true,
            ..Default::default()
        });
        p.quota = Some(QuotaRules::default());
        assert_eq!(p.validate(), None);
    }

    /// A row that declares nothing is not a policy.
    #[test]
    fn a_row_declaring_nothing_is_empty() {
        assert!(StoredPolicy::new("npm1", NodeKind::Package, "pkg").is_empty());
        let mut p = StoredPolicy::new("npm1", NodeKind::Package, "pkg");
        p.visibility = Some(Visibility::Team);
        assert!(!p.is_empty());
    }
}
