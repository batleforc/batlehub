use chrono::{DateTime, Utc};

use super::Visibility;

/// A team namespace claim: a group from the auth provider that owns a
/// slash-separated package prefix within a registry (e.g. `"frontend"`
/// owns packages whose name starts with `"frontend/"`).
#[derive(Debug, Clone)]
pub struct TeamNamespace {
    pub registry: String,
    /// Prefix without trailing slash (e.g. `"frontend"`).
    pub prefix: String,
    /// Auth-provider group name that must appear in `Identity.groups`.
    pub group_id: String,
    pub claimed_by: Option<String>,
    /// The character that separates this prefix from what is under it.
    ///
    /// # Why the claim carries it rather than the registry
    ///
    /// RFC 0015 §4.1 makes the separator a property of the **ecosystem** — `/`
    /// for npm scopes and Go modules, `.` for OpenVSX publishers and NuGet ids,
    /// `:` for Maven groupIds — and every matcher in this tree hardcoded `/`.
    /// On a dotted ecosystem that made a claim on `digital` match `digital` and
    /// nothing else, which is RFC 0011-bis §4.2's bug from the other side: there
    /// a prefix matched too much, here it matches too little.
    ///
    /// It is stored on the claim rather than looked up per query for one reason
    /// that outweighs the tidiness of deriving it: **`LOCAL_VISIBILITY_PREDICATE`
    /// and `find_namespace` have to agree character for character** (§6.3), and
    /// the explore predicate runs across many registries at once, so a
    /// per-registry lookup would have to be threaded into SQL as a parallel array
    /// and joined. Reading one column makes the two agree by construction instead
    /// of by care.
    ///
    /// Existing rows default to `/`, which is exactly what they matched before —
    /// §10's promise. A claim made on a dotted ecosystem *before* this column
    /// existed keeps its old, narrower matching until it is re-claimed; that is
    /// the conservative direction and it is not a regression, because it is what
    /// the row already did.
    pub separator: char,
}

impl TeamNamespace {
    /// Whether `package` lies under this claim.
    ///
    /// **The one matcher.** `find_namespace`'s SQL and the in-memory store both
    /// answer the same question, and `LOCAL_VISIBILITY_PREDICATE` answers it a
    /// third time in SQL — §6.3 requires all of them to agree, and the way three
    /// implementations of one rule stay in agreement is for two of them to be
    /// tested against the third rather than reasoned about.
    ///
    /// Equality counts (a namespace contains itself) and anything deeper must be
    /// separated by this claim's own character, so `digital` never matches
    /// `digitalpipeline`.
    pub fn covers(&self, package: &str) -> bool {
        if package == self.prefix {
            return true;
        }
        package
            .strip_prefix(&self.prefix)
            .is_some_and(|rest| rest.starts_with(self.separator))
    }
}

/// A single published package version within a team namespace.
#[derive(Debug, Clone)]
pub struct NamespacePackage {
    pub name: String,
    pub version: String,
    pub visibility: Visibility,
    pub published_by: String,
    pub published_at: DateTime<Utc>,
    pub yanked: bool,
}
