use super::Deserialize;

// ── Serde types for repodata.json ─────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub(super) struct CondaRepodata {
    #[serde(default)]
    pub(super) packages: std::collections::HashMap<String, CondaPackageEntry>,
    #[serde(default, rename = "packages.conda")]
    pub(super) packages_conda: std::collections::HashMap<String, CondaPackageEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CondaPackageEntry {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) version: Option<String>,
    #[serde(default)]
    pub(super) build: Option<String>,
    #[serde(default)]
    pub(super) sha256: Option<String>,
    /// Build timestamp in milliseconds since the Unix epoch.
    /// Present in most but not all repodata.json entries.
    #[serde(default)]
    pub(super) timestamp: Option<i64>,
}

/// Metadata extracted from a conda package archive.
#[derive(Debug, Clone)]
pub struct CondaPackageInfo {
    pub name: String,
    pub version: String,
    pub build: String,
    pub build_number: u64,
    pub depends: Vec<String>,
    pub subdir: Option<String>,
    pub license: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CondaIndexJson {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) build: String,
    #[serde(default)]
    pub(super) build_number: u64,
    #[serde(default)]
    pub(super) depends: Vec<String>,
    #[serde(default)]
    pub(super) subdir: Option<String>,
    #[serde(default)]
    pub(super) license: Option<String>,
}
