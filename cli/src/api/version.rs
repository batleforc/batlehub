use anyhow::Result;
use serde::Serialize;

use super::BatleHubClient;

#[derive(Debug, Serialize)]
struct BulkVersionRequest {
    packages: Vec<PackageRef>,
}

#[derive(Debug, Serialize)]
struct PackageRef {
    name: String,
    version: String,
}

impl BatleHubClient {
    pub async fn yank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/bulk-yank"),
            &body,
        )
        .await
    }

    pub async fn unyank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/bulk-unyank"),
            &body,
        )
        .await
    }

    pub async fn delete_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/bulk-delete"),
            &body,
        )
        .await
    }
}
