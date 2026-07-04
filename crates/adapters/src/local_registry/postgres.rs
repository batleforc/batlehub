use std::time::Duration;

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::{entities::PublishedPackage, error::CoreError, ports::LocalRegistryBackend};

pub struct PostgresLocalRegistry {
    pool: PgPool,
}

impl PostgresLocalRegistry {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LocalRegistryBackend for PostgresLocalRegistry {
    /// Insert the row with `status = 'pending'` so the version is reserved but
    /// invisible to readers until `commit_publish` promotes it.
    ///
    /// Any stale pending row from a previous crashed publish is removed first so
    /// the caller can retry without hitting the unique constraint.
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
        // Remove a stale pending row if one exists (crash recovery for the caller).
        sqlx::query(
            "DELETE FROM local_packages \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'pending'",
        )
        .bind(&pkg.registry)
        .bind(&pkg.name)
        .bind(&pkg.version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        sqlx::query(
            "INSERT INTO local_packages \
                (registry, name, version, checksum, yanked, index_metadata, \
                 published_at, published_by, status, signature_bytes, signature_type, visibility) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10, $11)",
        )
        .bind(&pkg.registry)
        .bind(&pkg.name)
        .bind(&pkg.version)
        .bind(&pkg.checksum)
        .bind(pkg.yanked)
        .bind(&pkg.index_metadata)
        .bind(pkg.published_at)
        .bind(&pkg.published_by)
        .bind(&pkg.signature_bytes)
        .bind(&pkg.signature_type)
        .bind(pkg.visibility.to_string())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db) = e {
                if db.constraint() == Some("uq_local_package") {
                    return CoreError::Conflict(format!(
                        "{}@{} already published in registry '{}'",
                        pkg.name, pkg.version, pkg.registry
                    ));
                }
            }
            CoreError::Database(e.to_string())
        })?;
        Ok(())
    }

    async fn commit_publish(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages SET status = 'published' \
             WHERE registry = $1 AND name = $2 AND version = $3",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages \
             SET yanked = TRUE, \
                 index_metadata = jsonb_set(index_metadata, '{yanked}', 'true') \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn unyank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages \
             SET yanked = FALSE, \
                 index_metadata = jsonb_set(index_metadata, '{yanked}', 'false') \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn deprecate(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        message: Option<&str>,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages \
             SET deprecated = TRUE, \
                 deprecation_message = $4, \
                 index_metadata = jsonb_set(index_metadata, '{deprecated}', \
                     to_jsonb(COALESCE($4::text, 'true'))) \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .bind(message)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn undeprecate(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages \
             SET deprecated = FALSE, \
                 deprecation_message = NULL, \
                 index_metadata = index_metadata - 'deprecated' \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn unlist(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages SET unlisted = TRUE \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn relist(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        sqlx::query(
            "UPDATE local_packages SET unlisted = FALSE \
             WHERE registry = $1 AND name = $2 AND version = $3 AND status = 'published'",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn get_versions(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let rows = sqlx::query(
            "SELECT registry, name, version, checksum, yanked, deprecated, \
                    deprecation_message, unlisted, index_metadata, \
                    published_at, published_by, signature_bytes, signature_type, visibility \
             FROM local_packages \
             WHERE registry = $1 AND name = $2 AND status = 'published' \
             ORDER BY published_at ASC",
        )
        .bind(registry)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        rows.into_iter()
            .map(|r| {
                let vis = r
                    .get::<String, _>("visibility")
                    .parse()
                    .map_err(|e| CoreError::Database(format!("invalid visibility in db: {e}")))?;
                Ok(PublishedPackage {
                    registry: r.get("registry"),
                    name: r.get("name"),
                    version: r.get("version"),
                    checksum: r.get("checksum"),
                    yanked: r.get("yanked"),
                    deprecated: r.get("deprecated"),
                    deprecation_message: r.get("deprecation_message"),
                    unlisted: r.get("unlisted"),
                    index_metadata: r.get("index_metadata"),
                    published_at: r.get("published_at"),
                    published_by: r.get("published_by"),
                    signature_bytes: r.get("signature_bytes"),
                    signature_type: r.get("signature_type"),
                    visibility: vis,
                })
            })
            .collect()
    }

    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
        let row = sqlx::query(
            "SELECT 1 FROM local_packages \
             WHERE registry = $1 AND name = $2 AND status = 'published' \
             LIMIT 1",
        )
        .bind(registry)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(row.is_some())
    }

    async fn remove_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "DELETE FROM local_packages \
             WHERE registry = $1 AND name = $2 AND version = $3",
        )
        .bind(registry)
        .bind(name)
        .bind(version)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(())
    }

    async fn cleanup_pending(&self, older_than: Duration) -> Result<u64, CoreError> {
        let secs = older_than.as_secs() as i64;
        let result = sqlx::query(
            "DELETE FROM local_packages \
             WHERE status = 'pending' \
               AND published_at < NOW() - ($1 || ' seconds')::INTERVAL",
        )
        .bind(secs)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(result.rows_affected())
    }

    async fn list_package_names(&self, registry: &str) -> Result<Vec<String>, CoreError> {
        let rows = sqlx::query(
            "SELECT DISTINCT name FROM local_packages \
             WHERE registry = $1 AND status = 'published' \
             ORDER BY name ASC",
        )
        .bind(registry)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| r.get::<String, _>("name"))
            .collect())
    }
}
