use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::BatleHubClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub registry_type: String,
    pub mode: String,
}

impl BatleHubClient {
    pub async fn list_registries(&self) -> Result<Vec<RegistryInfo>> {
        self.get("/api/v1/registries").await
    }
}
