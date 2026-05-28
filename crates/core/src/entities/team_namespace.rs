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
}
