use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A package published directly to this BatleHub instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedPackage {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// SHA-256 hex of the artifact bytes.
    pub checksum: String,
    pub yanked: bool,
    /// Registry-specific index line as opaque JSON.
    /// For Cargo: serialised `CargoIndexEntry`.
    pub index_metadata: serde_json::Value,
    pub published_at: DateTime<Utc>,
    pub published_by: Option<String>,
}

/// One newline-delimited line in a Cargo sparse index file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoIndexEntry {
    pub name: String,
    pub vers: String,
    pub deps: Vec<CargoDep>,
    pub cksum: String,
    pub features: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features2: Option<serde_json::Value>,
    pub yanked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoDep {
    pub name: String,
    /// Version requirement string (e.g. `"^1.0"`).
    pub req: String,
    pub features: Vec<String>,
    pub optional: bool,
    pub default_features: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// `"normal"`, `"dev"`, or `"build"`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_name_in_toml: Option<String>,
}
