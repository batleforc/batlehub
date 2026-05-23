use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Uniquely identifies a package (or sub-artifact) in a registry.
///
/// Examples:
/// - GitHub release asset: `{ registry: "github", name: "rust-lang/rust", version: "v1.80.0", artifact: Some("12345678") }`
/// - Cargo crate:          `{ registry: "cargo",  name: "tokio",           version: "1.38.0",  artifact: None }`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PackageId {
    pub registry: String,
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

impl PackageId {
    pub fn new(registry: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            registry: registry.into(),
            name: name.into(),
            version: version.into(),
            artifact: None,
        }
    }

    pub fn with_artifact(mut self, artifact: impl Into<String>) -> Self {
        self.artifact = Some(artifact.into());
        self
    }

    /// Stable string key suitable for use as a cache or storage key.
    pub fn cache_key(&self) -> String {
        match &self.artifact {
            Some(art) => format!("{}/{}/{}/{}", self.registry, self.name, self.version, art),
            None => format!("{}/{}/{}", self.registry, self.name, self.version),
        }
    }
}

impl std::fmt::Display for PackageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cache_key())
    }
}

/// Metadata fetched from an upstream registry about a package/release.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    pub id: PackageId,
    pub published_at: Option<DateTime<Utc>>,
    /// Direct URL to download the artifact (if applicable).
    pub download_url: Option<String>,
    /// Content hash of the artifact (SHA-256, hex-encoded).
    pub checksum: Option<String>,
    /// Whether this artifact has a detached signature (.asc / .sig).
    pub is_signed: Option<bool>,
    /// Registry-specific extra fields (e.g., GitHub release body, Cargo license).
    pub extra: Value,
    /// Raw `Cache-Control` header value from the upstream metadata response, if any.
    /// Used to apply `no-store`, `no-cache`, or `max-age` directives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<String>,
}

/// Administrative status of a package in this proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PackageStatus {
    Available,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: DateTime<Utc>,
    },
}

impl PackageStatus {
    pub fn is_blocked(&self) -> bool {
        matches!(self, PackageStatus::Blocked { .. })
    }
}

/// Lightweight summary used in listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSummary {
    pub id: Uuid,
    pub package_id: PackageId,
    pub status: PackageStatus,
    pub last_accessed: Option<DateTime<Utc>>,
    /// User who last successfully downloaded this package. `None` means anonymous.
    pub last_accessed_by: Option<String>,
    pub access_count: u64,
}

/// Filter for listing packages.
#[derive(Debug, Clone, Default)]
pub struct PackageFilter {
    pub registry: Option<String>,
    pub name_contains: Option<String>,
    /// Exact match on `package_name` — takes priority over `name_contains`.
    pub name_exact: Option<String>,
    pub blocked_only: bool,
    pub limit: u64,
    pub offset: u64,
}

impl PackageFilter {
    pub fn new() -> Self {
        Self {
            limit: 50,
            ..Default::default()
        }
    }
}
