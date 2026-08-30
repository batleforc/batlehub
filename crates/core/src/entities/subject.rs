//! Who is asking, and what they are asking about.
//!
//! RFC 0015 §5.1 gives the decision function three inputs — a subject, an
//! action, and a resource. [`Action`](super::Action) landed in phase 1; this is
//! the other two.
//!
//! # Why a `Subject` rather than an `Identity`
//!
//! An [`Identity`] is what an auth provider produced: a user id, a role, a
//! provider, some groups. A [`Subject`] is what authorization judges, and the
//! two are not the same thing for long. RFC 0012's signed URLs already redeem
//! into something that is allowed to fetch one coordinate and is nobody in
//! particular; §4.3's `token:<name>` is a machine credential with no user behind
//! it. Both are subjects and neither is comfortably an `Identity`.
//!
//! Today the only variant is [`Subject::Identity`], and that is the honest state
//! of phase 2 — the type exists so the ones that follow are added beside it
//! rather than threaded through every signature afterwards.

use super::{Identity, PackageId, Role};

/// The principal a decision is made about.
#[derive(Debug, Clone)]
pub enum Subject {
    /// A resolved caller — the only form today.
    Identity(Identity),
}

impl Subject {
    /// The underlying identity, while every subject still has one.
    ///
    /// This is the seam. Every rule in the chain takes an `&Identity`, and phase
    /// 2 does not change that: `authorize` is one entry point over today's
    /// evaluation, not a rewrite of it. When a subject arrives that has no
    /// identity behind it, this becomes the place the compiler makes that
    /// visible rather than a `.unwrap()` somewhere further in.
    pub fn identity(&self) -> &Identity {
        match self {
            Subject::Identity(id) => id,
        }
    }

    /// Whether this subject is an administrator.
    pub fn is_admin(&self) -> bool {
        self.identity().is_admin()
    }

    /// Whether this subject holds at least `role`.
    pub fn has_role_at_least(&self, role: &Role) -> bool {
        self.identity().has_role_at_least(role)
    }
}

impl From<Identity> for Subject {
    fn from(id: Identity) -> Self {
        Subject::Identity(id)
    }
}

impl From<&Identity> for Subject {
    fn from(id: &Identity) -> Self {
        Subject::Identity(id.clone())
    }
}

/// Which tier of the hierarchy a decision is about.
///
/// RFC 0015 §4.1's four tiers. Phase 2 judges the same coordinates it always
/// did — this names them, so the code that walks the hierarchy in phase 3 is
/// added to a shape that already exists rather than replacing one that does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// The whole server, above every registry.
    ///
    /// RFC 0015 §4.1's hierarchy starts at `Registry`, which is right for
    /// everything that names a package — and is why decomposing `require_admin`
    /// was deferred: about a dozen control endpoints (config, health, the
    /// notification wiring, the IP and account blocks, the authorization
    /// diagnostics) name **no registry at all**, so there was no node their
    /// grants could attach to.
    ///
    /// This is that node. It sits above `Registry` on every path, so a grant
    /// written here reaches everything beneath it by the same union §4.3 already
    /// defines — no new composition rule, and one more tier in `tiers_walked`.
    Instance,
    Registry,
    Namespace,
    Package,
    Version,
}

/// What a decision is about.
///
/// A [`PackageId`] carries registry, name and version, which is a *version*-tier
/// coordinate. The shallower tiers are the same coordinate with the deeper parts
/// unasked-about: a listing names a package and no version, and a whole-registry
/// document names neither.
///
/// The distinction is not decorative. It is why `authorize_listing` exists as a
/// separate function today — a listing has no concrete version, and handing one
/// to a chain that reads `published_at` does not gate the listing, it blanks it
/// (see `services::authz`). Phase 2 records the tier on the resource instead of
/// encoding it in which function you called.
#[derive(Debug, Clone)]
pub struct Resource {
    /// The coordinate. For tiers above `Version` the version field is a
    /// placeholder the tier says to ignore.
    pub id: PackageId,
    /// How much of `id` is being asked about.
    pub tier: Tier,
}

impl Resource {
    /// A whole registry — a search index, a catalogue, a channel document.
    pub fn registry(registry: impl Into<String>) -> Self {
        Resource {
            id: PackageId::new(registry.into(), String::new(), String::new()),
            tier: Tier::Registry,
        }
    }

    /// One package, no version named — a version listing.
    pub fn package(registry: impl Into<String>, name: impl Into<String>) -> Self {
        Resource {
            id: PackageId::new(registry.into(), name.into(), String::new()),
            tier: Tier::Package,
        }
    }

    /// One concrete version.
    pub fn version(id: PackageId) -> Self {
        Resource {
            id,
            tier: Tier::Version,
        }
    }

    pub fn registry_name(&self) -> &str {
        self.id.registry.as_str()
    }
}

impl From<&PackageId> for Resource {
    fn from(id: &PackageId) -> Self {
        Resource::version(id.clone())
    }
}

/// The answer.
///
/// Two variants, and there will not be a third — see `RuleDecision` in
/// `crate::rules` for what the third one cost.
#[derive(Debug, Clone)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }

    /// The reason, when there is one.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Decision::Allow => None,
            Decision::Deny { reason } => Some(reason),
        }
    }

    /// Fold into the `Result` the read paths already speak.
    pub fn into_result(self) -> Result<(), crate::error::CoreError> {
        match self {
            Decision::Allow => Ok(()),
            Decision::Deny { reason } => Err(crate::error::CoreError::AccessDenied(reason)),
        }
    }
}

impl From<crate::rules::RuleDecision> for Decision {
    fn from(d: crate::rules::RuleDecision) -> Self {
        match d {
            crate::rules::RuleDecision::Allow => Decision::Allow,
            crate::rules::RuleDecision::Deny { reason } => Decision::Deny { reason },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_remembers_which_tier_it_names() {
        assert_eq!(Resource::registry("reg").tier, Tier::Registry);
        assert_eq!(Resource::package("reg", "pkg").tier, Tier::Package);
        assert_eq!(
            Resource::version(PackageId::new("reg", "pkg", "1.0.0")).tier,
            Tier::Version
        );
    }

    /// Tiers order outermost-first, which is the order grant resolution walks
    /// them in (RFC 0015 §4.3).
    #[test]
    fn tiers_order_from_registry_inwards() {
        assert!(Tier::Registry < Tier::Namespace);
        assert!(Tier::Namespace < Tier::Package);
        assert!(Tier::Package < Tier::Version);
    }

    #[test]
    fn a_denial_carries_its_reason_into_the_error() {
        let err = Decision::Deny {
            reason: "nope".to_owned(),
        }
        .into_result()
        .expect_err("a denial is an error");
        assert!(matches!(err, crate::error::CoreError::AccessDenied(r) if r == "nope"));
        assert!(Decision::Allow.into_result().is_ok());
    }
}
