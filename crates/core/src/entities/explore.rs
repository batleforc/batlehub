use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Where a package entry originates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageSource {
    Proxied,
    Local,
    Both,
}

/// Sort order for explore queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExploreSortBy {
    #[default]
    Downloads,
    Name,
    Recent,
}

/// One collapsed entry per (registry, package_name) in the explorer listing.
#[derive(Debug, Clone)]
pub struct ExploreEntry {
    pub registry: String,
    pub name: String,
    pub version_count: u64,
    pub total_downloads: u64,
    pub last_accessed: Option<DateTime<Utc>>,
    pub source: PackageSource,
    pub has_blocked: bool,
}

/// Who is asking, for the purpose of per-package visibility filtering.
///
/// Registry-level access is a separate, coarser gate handled by
/// `ExploreFilter::registries`. This type carries what is needed to apply the
/// *package*-level `Visibility` rule — the same rule
/// `LocalRegistryService::check_visibility` enforces on the download path — to a
/// listing, so an `internal`/`team` package does not surface its name and version
/// count to someone who could never download it.
///
/// [`Default`] is the anonymous viewer: not an admin, not authenticated, no
/// groups. That is the safe default for a filter built without thinking about
/// it — it sees only `public` packages.
#[derive(Debug, Clone, Default)]
pub struct ExploreViewer {
    /// Admins bypass visibility entirely, matching `check_visibility`.
    pub is_admin: bool,
    /// Any non-anonymous identity. Gates `Visibility::Internal`.
    pub is_authenticated: bool,
    /// The caller's group memberships, used for `Visibility::Team`.
    pub groups: Vec<String>,
}

impl ExploreViewer {
    /// Group ids with spaces removed, matching how `check_team_visibility`
    /// compares them (`g.replace(' ', "")`). Pre-normalising here keeps the
    /// comparison identical on the SQL side, where doing it per row would be
    /// both slower and easy to get subtly different.
    pub fn normalised_groups(&self) -> Vec<String> {
        self.groups.iter().map(|g| g.replace(' ', "")).collect()
    }
}

/// Filter for explore queries.
#[derive(Debug, Clone, Default)]
pub struct ExploreFilter {
    pub registry: Option<String>,
    /// Access-control allow-list. Empty means "all accessible registries".
    pub registries: Vec<String>,
    pub name_contains: Option<String>,
    pub sort_by: ExploreSortBy,
    pub limit: u64,
    pub offset: u64,
    /// Package-level visibility gate. See [`ExploreViewer`].
    pub viewer: ExploreViewer,
}

/// Per-registry statistics for the explorer sidebar.
#[derive(Debug, Clone)]
pub struct RegistryStat {
    pub registry: String,
    pub package_count: u64,
    pub total_downloads: u64,
}

/// Full package detail returned by the explorer detail endpoint.
#[derive(Debug, Clone)]
pub struct ExplorePackageDetail {
    pub registry: String,
    pub name: String,
    pub gate: GateInfo,
    pub versions: Vec<ExploreVersionEntry>,
}

/// Gate/access status for a package's registry.
#[derive(Debug, Clone)]
pub struct GateInfo {
    /// Whether the caller's role can access this registry through the proxy.
    pub registry_accessible: bool,
    /// Whether the caller is a beta-channel member for this registry.
    pub beta_member: bool,
}

/// One version of a package, with source and firewall status.
#[derive(Debug, Clone)]
pub struct ExploreVersionEntry {
    pub version: String,
    pub source: String,
    pub firewall: FirewallInfo,
    pub download_count: u64,
    pub last_accessed: Option<DateTime<Utc>>,
    pub published_at: Option<DateTime<Utc>>,
    /// `true` when the version string contains a `-` (semver pre-release component).
    pub is_prerelease: bool,
}

/// Firewall / blocking status of a single package version.
#[derive(Debug, Clone)]
pub enum FirewallInfo {
    Clear,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: DateTime<Utc>,
    },
    Yanked,
}
