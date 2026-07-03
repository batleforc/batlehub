use serde::Deserialize;

// ── Serde types for GitHub API responses ─────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GhRelease {
    pub id: u64,
    pub tag_name: String,
    pub published_at: Option<String>,
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GhAsset {
    pub id: u64,
    pub name: String,
    pub browser_download_url: String,
    #[allow(dead_code)]
    pub size: u64,
}
