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
}
