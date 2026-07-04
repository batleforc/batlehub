use anyhow::Result;

use super::admin::BulkPackageResult;
use super::BatleHubClient;

/// Report the first per-package failure from a bulk-endpoint call as an error,
/// matching the single-package yank/unyank/delete contract these bulk calls
/// stand in for.
fn require_success(result: BulkPackageResult) -> Result<()> {
    if let Some(f) = result.failed.first() {
        anyhow::bail!("{}/{}: {}", f.name, f.version, f.error);
    }
    Ok(())
}

impl BatleHubClient {
    /// Yanks/unyanks/deletes call their bulk-endpoint counterpart with a
    /// single-package list rather than maintaining a parallel single-package
    /// DTO and response shape, so there is exactly one client-side
    /// implementation of each bulk-* endpoint.
    pub async fn yank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let result = self
            .bulk_yank(registry, vec![(name.to_string(), version.to_string())])
            .await?;
        require_success(result)
    }

    pub async fn unyank_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let result = self
            .bulk_unyank(registry, vec![(name.to_string(), version.to_string())])
            .await?;
        require_success(result)
    }

    pub async fn delete_version(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        let result = self
            .bulk_delete(registry, vec![(name.to_string(), version.to_string())])
            .await?;
        require_success(result)
    }
}
