use serde::Deserialize;
use std::collections::HashMap;

// ── Serde types (internal) ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct PackagistV2Response {
    pub(super) packages: HashMap<String, Vec<ComposerVersionEntry>>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct ComposerVersionEntry {
    pub(super) version: String,
    pub(super) dist: Option<ComposerDist>,
    pub(super) time: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub(super) struct ComposerDist {
    pub(super) url: String,
    pub(super) shasum: Option<String>,
    pub(super) reference: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ComposerJson {
    pub(super) name: String,
    pub(super) version: Option<String>,
    pub(super) description: Option<String>,
}

// ── Public types ──────────────────────────────────────────────────────────────

/// Metadata extracted from a Composer package ZIP on publish.
pub struct ComposerPackageMeta {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub composer_json: serde_json::Value,
}
