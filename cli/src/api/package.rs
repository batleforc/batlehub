use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::BatleHubClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageListResponse {
    pub items: Vec<PackageSummary>,
    pub total: usize,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSummary {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub status: PackageStatus,
    pub access_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PackageStatus {
    Available,
    Blocked { reason: String },
}

#[derive(Debug, Serialize)]
pub struct PackageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub page: u64,
    pub per_page: u64,
}

impl BatleHubClient {
    pub async fn list_packages(&self, query: PackageQuery) -> Result<PackageListResponse> {
        self.get_with_params("/api/v1/packages", &query).await
    }
}
