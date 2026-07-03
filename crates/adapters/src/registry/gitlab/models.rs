use serde::Deserialize;

use batlehub_core::error::CoreError;

use super::client::GitlabRegistryClient;

// ── Serde types for GitLab API responses ──────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GlRelease {
    pub tag_name: String,
    /// GitLab uses `released_at` (not `published_at`).
    pub released_at: Option<String>,
    #[serde(default)]
    pub assets: GlAssets,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GlAssets {
    #[serde(default)]
    pub links: Vec<GlLink>,
    #[serde(default)]
    pub sources: Vec<GlSource>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GlLink {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub direct_asset_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GlSource {
    pub format: String,
    pub url: String,
}

impl GitlabRegistryClient {
    /// Resolve a release link asset to its upstream download URL, matched by the
    /// link `name`.
    pub(super) async fn link_download_url(
        &self,
        project: &str,
        tag: &str,
        name: &str,
    ) -> Result<String, CoreError> {
        let release = self.fetch_release_by_tag(project, tag).await?;
        release
            .assets
            .links
            .iter()
            .find(|l| l.name == name)
            // Prefer the direct asset URL (stable permalink) when present.
            .map(|l| l.direct_asset_url.clone().unwrap_or_else(|| l.url.clone()))
            .ok_or_else(|| {
                CoreError::NotFound(format!("no release link named '{name}' in {project}@{tag}"))
            })
    }
}
