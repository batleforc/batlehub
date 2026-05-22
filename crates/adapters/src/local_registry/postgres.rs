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
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
        sqlx::query(
            r#"
            INSERT INTO local_packages
                (registry, name, version, checksum, yanked, index_metadata,
                 published_at, published_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(&pkg.registry)
        .bind(&pkg.name)
        .bind(&pkg.version)
        .bind(&pkg.checksum)
        .bind(pkg.yanked)
        .bind(&pkg.index_metadata)
        .bind(pkg.published_at)
        .bind(&pkg.published_by)
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

    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        sqlx::query(
            r#"
            UPDATE local_packages
            SET yanked = TRUE,
                index_metadata = jsonb_set(index_metadata, '{yanked}', 'true')
            WHERE registry = $1 AND name = $2 AND version = $3
            "#,
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
            r#"
            UPDATE local_packages
            SET yanked = FALSE,
                index_metadata = jsonb_set(index_metadata, '{yanked}', 'false')
            WHERE registry = $1 AND name = $2 AND version = $3
            "#,
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
            r#"
            SELECT registry, name, version, checksum, yanked, index_metadata,
                   published_at, published_by
            FROM local_packages
            WHERE registry = $1 AND name = $2
            ORDER BY published_at ASC
            "#,
        )
        .bind(registry)
        .bind(name)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| PublishedPackage {
                registry: r.get("registry"),
                name: r.get("name"),
                version: r.get("version"),
                checksum: r.get("checksum"),
                yanked: r.get("yanked"),
                index_metadata: r.get("index_metadata"),
                published_at: r.get("published_at"),
                published_by: r.get("published_by"),
            })
            .collect())
    }

    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
        let row = sqlx::query(
            "SELECT 1 FROM local_packages WHERE registry = $1 AND name = $2 LIMIT 1",
        )
        .bind(registry)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::Database(e.to_string()))?;
        Ok(row.is_some())
    }
}
