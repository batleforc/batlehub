use async_trait::async_trait;

use batlehub_core::{error::CoreError, ports::UpstreamSbomFetcher};

/// HTTP-based upstream SBOM fetcher.
///
/// Supports:
/// - GitHub: `GET /repos/{owner}/{repo}/dependency-graph/sbom`
/// - npm: checks version metadata for a `bom` field
pub struct HttpSbomFetcher {
    client: reqwest::Client,
}

impl HttpSbomFetcher {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl UpstreamSbomFetcher for HttpSbomFetcher {
    async fn fetch(
        &self,
        registry_type: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<serde_json::Value>, CoreError> {
        match registry_type {
            "github" => fetch_github_sbom(&self.client, name).await,
            "npm" => fetch_npm_bom(&self.client, name, version).await,
            _ => Ok(None),
        }
    }
}

async fn fetch_github_sbom(
    client: &reqwest::Client,
    name: &str,
) -> Result<Option<serde_json::Value>, CoreError> {
    // name is "{owner}/{repo}" for the GitHub registry
    let url = format!("https://api.github.com/repos/{name}/dependency-graph/sbom");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| CoreError::Registry(e.to_string()))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CoreError::Registry(e.to_string()))?;

    Ok(json.get("sbom").cloned())
}

async fn fetch_npm_bom(
    client: &reqwest::Client,
    name: &str,
    version: &str,
) -> Result<Option<serde_json::Value>, CoreError> {
    let url = format!("https://registry.npmjs.org/{name}/{version}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| CoreError::Registry(e.to_string()))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CoreError::Registry(e.to_string()))?;

    Ok(json.get("bom").cloned())
}
