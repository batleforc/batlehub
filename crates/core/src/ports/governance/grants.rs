//! Storage for the two grant tiers a config file cannot express.
//!
//! RFC 0015 §4.1: the registry and namespace tiers live in TOML and are built
//! at load; the package and version tiers cannot, because "a registry with
//! 200 000 packages will not enumerate them in TOML, let alone their two
//! million versions". Those two are written through the admin API and read from
//! here.
//!
//! # There is no seal in this port
//!
//! Deliberately, and it is the port's most important property. §4.3 confines
//! sealing to the config file: it is the one construct that takes access away,
//! and a delegate holding `owners:write` may write package and version rows — so
//! a seal representable here would let them lock the registry owner out of a
//! package, which is revocation reintroduced one tier below the model built to
//! exclude it.
//!
//! §7 asks for that to be "not a rejected request but an unwritable one", so the
//! type carries no way to say it: a [`StoredGrant`] always has a subject and a
//! non-empty action set, and the empty grant map that *is* a seal has no
//! representation. `crates/core`'s tests assert the type cannot express one.

use async_trait::async_trait;

use crate::entities::{Action, SubjectMatcher};
use crate::error::CoreError;

/// Which of the two stored tiers a row is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Package,
    Version,
}

impl NodeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            NodeKind::Package => "package",
            NodeKind::Version => "version",
        }
    }
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for NodeKind {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "package" => Ok(NodeKind::Package),
            "version" => Ok(NodeKind::Version),
            other => Err(CoreError::InvalidInput(format!(
                "unknown grant node kind '{other}'"
            ))),
        }
    }
}

/// The key a node is addressed by within its registry.
///
/// A package node is keyed by its name; a version node by `name@version`. One
/// key rather than two columns, because the pair is only ever read whole —
/// resolution asks "what is written on this exact node", never "every version
/// row of this package regardless of which".
pub fn version_node_key(package: &str, version: &str) -> String {
    format!("{package}@{version}")
}

/// One row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGrant {
    pub registry: String,
    pub node_kind: NodeKind,
    pub node_key: String,
    pub subject: SubjectMatcher,
    /// Already expanded (§4.2: expansion happens at write, never at
    /// evaluation). A stored `releases:*` would move that decision back onto
    /// every request.
    pub actions: Vec<Action>,
    pub granted_by: Option<String>,
}

#[async_trait]
pub trait GrantRepository: Send + Sync {
    /// Every stored grant on the package and version nodes for one coordinate.
    ///
    /// **One call, both tiers.** Resolution needs them together and a read that
    /// took two round trips would double the cost §11.7 measures against a 2 ms
    /// p99 budget.
    ///
    /// `version` is optional: a listing names a package and no version, and
    /// asking for version rows would return grants for a coordinate the caller
    /// did not name.
    async fn grants_for(
        &self,
        registry: &str,
        package: &str,
        version: Option<&str>,
    ) -> Result<Vec<StoredGrant>, CoreError>;

    /// Write one subject's grant on a node, replacing that subject's row.
    ///
    /// Replacing *that subject's* row rather than the node's other rows: two
    /// rows for one subject would make the union depend on which was read
    /// first, and repeating a subject is a union in the model rather than a
    /// second opinion.
    async fn put_grant(&self, grant: StoredGrant) -> Result<(), CoreError>;

    /// Remove one subject's grant from a node. Absent is not an error.
    async fn delete_grant(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
        subject: &SubjectMatcher,
    ) -> Result<(), CoreError>;

    /// Every **package-tier** grant in a registry.
    ///
    /// For the one case that needs it: filtering a whole-registry document for a
    /// caller whose broad tiers do not grant the read (§4.4). Resolving per
    /// package would be the N+1 that measured 806× at size M (§13.2); this is
    /// one query, and the rows are few because a package-tier grant is something
    /// an operator wrote deliberately.
    ///
    /// Not called on the fast path. A caller who holds `releases:read` at the
    /// registry or namespace tier holds it on every package beneath — grants
    /// only widen — so there is nothing to filter and nothing to fetch.
    async fn package_grants_in_registry(
        &self,
        registry: &str,
    ) -> Result<Vec<StoredGrant>, CoreError>;

    /// Every grant on one node, for the admin API and `explain`.
    async fn grants_on_node(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<Vec<StoredGrant>, CoreError>;

    /// Drop every grant keyed by a package name, at both tiers.
    ///
    /// RFC 0016 §4.4, quoted in §12: **package-tier policy dies with the
    /// package.** Deleting a package's last version deletes its grants, because
    /// grants keyed by a name that outlive the package would leave a previous
    /// owner holding `releases:publish` on a name someone else may take —
    /// survey finding 1's stale-claim shape arriving through the back door.
    ///
    /// The version tier goes too: those keys are prefixed by the same package
    /// name and are equally stale.
    async fn delete_package_grants(&self, registry: &str, package: &str) -> Result<(), CoreError>;
}
