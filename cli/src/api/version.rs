use anyhow::Result;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
struct BulkPackageFailure {
    name: String,
    version: String,
    error: String,
}

#[derive(Debug, Deserialize)]
struct BulkPackageResponse {
    failed: Vec<BulkPackageFailure>,
}

impl BatleHubClient {
    pub async fn yank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        let resp: BulkPackageResponse = self
            .post(
                &format!("/api/v1/admin/registries/{registry}/bulk-yank"),
                &body,
            )
            .await?;
        if let Some(f) = resp.failed.first() {
            anyhow::bail!("{}/{}: {}", f.name, f.version, f.error);
        }
        Ok(())
    }

    pub async fn unyank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        let resp: BulkPackageResponse = self
            .post(
                &format!("/api/v1/admin/registries/{registry}/bulk-unyank"),
                &body,
            )
            .await?;
        if let Some(f) = resp.failed.first() {
            anyhow::bail!("{}/{}: {}", f.name, f.version, f.error);
        }
        Ok(())
    }

    pub async fn delete_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let body = BulkVersionRequest {
            packages: vec![PackageRef {
                name: name.to_string(),
                version: version.to_string(),
            }],
        };
        let resp: BulkPackageResponse = self
            .post(
                &format!("/api/v1/admin/registries/{registry}/bulk-delete"),
                &body,
            )
            .await?;
        if let Some(f) = resp.failed.first() {
            anyhow::bail!("{}/{}: {}", f.name, f.version, f.error);
        }
        Ok(())
    }
}
