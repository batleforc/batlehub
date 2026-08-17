use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct GemInfo {
    #[serde(default)]
    pub(super) version: String,
    #[serde(default)]
    pub(super) created_at: Option<String>,
    #[serde(default)]
    pub(super) sha: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GemVersion {
    pub(super) number: String,
}

/// Metadata extracted from a `.gem` archive.
#[derive(Debug, Clone)]
pub struct GemMetadata {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub summary: Option<String>,
    pub authors: Vec<String>,
    /// Runtime dependencies, in compact-index order.
    ///
    /// Captured at publish time because the compact index — the document
    /// Bundler resolves from — carries them inline, and a resolver handed an
    /// empty dependency list installs a gem without the gems it needs. Nothing
    /// read them before, because nothing generated a local compact index
    /// (RFC 0009 §12.15).
    pub dependencies: Vec<GemDependency>,
}

/// One runtime dependency of a gem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GemDependency {
    pub name: String,
    /// The requirement as the compact index writes it: constraints joined with
    /// `&`, e.g. `">= 1.0&< 2.0"`. Empty means "any version".
    pub requirement: String,
}
