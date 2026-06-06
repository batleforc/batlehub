use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::BatleHubClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerEntry {
    pub principal_type: String,
    pub principal_id: String,
    pub role: String,
    pub granted_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddOwnerRequest {
    pub principal_type: String,
    pub principal_id: String,
    pub role: String,
    pub granted_by: Option<String>,
}

impl BatleHubClient {
    pub async fn list_owners(&self, registry: &str, name: &str) -> Result<Vec<OwnerEntry>> {
        self.get(&format!(
            "/api/v1/admin/registries/{registry}/packages/{name}/owners"
        ))
        .await
    }

    pub async fn add_owner(&self, registry: &str, name: &str, req: AddOwnerRequest) -> Result<()> {
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/packages/{name}/owners"),
            &req,
        )
        .await
    }

    pub async fn remove_owner(
        &self,
        registry: &str,
        name: &str,
        principal_type: &str,
        principal_id: &str,
    ) -> Result<()> {
        self.delete(&format!(
            "/api/v1/admin/registries/{registry}/packages/{name}/owners/{principal_type}/{principal_id}"
        ))
        .await
    }
}
