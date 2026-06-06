use crate::db::DbResultExt;
use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::{
    entities::Identity,
    error::CoreError,
    ports::{OwnerEntry, OwnershipPort},
};

pub struct PgOwnershipStore {
    pool: PgPool,
}

impl PgOwnershipStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OwnershipPort for PgOwnershipStore {
    async fn initialize_owner(
        &self,
        registry: &str,
        package: &str,
        user_id: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO package_owners \
                (registry, package_name, principal_type, principal_id, role) \
             VALUES ($1, $2, 'user', $3, 'admin') \
             ON CONFLICT (registry, package_name, principal_type, principal_id) DO NOTHING",
        )
        .bind(registry)
        .bind(package)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn can_publish(
        &self,
        registry: &str,
        package: &str,
        identity: &Identity,
    ) -> Result<bool, CoreError> {
        // If there are no owners yet, any authenticated user may publish.
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM package_owners \
             WHERE registry = $1 AND package_name = $2",
        )
        .bind(registry)
        .bind(package)
        .fetch_one(&self.pool)
        .await
        .db_err()?;

        if count == 0 {
            return Ok(true);
        }

        // Check if the caller's user_id is an owner.
        if let Some(ref uid) = identity.user_id {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM package_owners \
                  WHERE registry = $1 AND package_name = $2 \
                    AND principal_type = 'user' AND principal_id = $3)",
            )
            .bind(registry)
            .bind(package)
            .bind(uid)
            .fetch_one(&self.pool)
            .await
            .db_err()?;
            if exists {
                return Ok(true);
            }
        }

        // Check if any of the caller's groups is an owner.
        if !identity.groups.is_empty() {
            let groups: Vec<&str> = identity.groups.iter().map(String::as_str).collect();
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM package_owners \
                  WHERE registry = $1 AND package_name = $2 \
                    AND principal_type = 'group' AND principal_id = ANY($3))",
            )
            .bind(registry)
            .bind(package)
            .bind(&groups)
            .fetch_one(&self.pool)
            .await
            .db_err()?;
            if exists {
                return Ok(true);
            }
        }

        Ok(false)
    }

    async fn add_owner(
        &self,
        registry: &str,
        package: &str,
        entry: OwnerEntry,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO package_owners \
                (registry, package_name, principal_type, principal_id, role, granted_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(registry)
        .bind(package)
        .bind(&entry.principal_type)
        .bind(&entry.principal_id)
        .bind(&entry.role)
        .bind(&entry.granted_by)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db) = e {
                if db.constraint() == Some("uq_package_owner") {
                    return CoreError::Conflict(format!(
                        "{} '{}' is already an owner of '{}/{}'",
                        entry.principal_type, entry.principal_id, registry, package
                    ));
                }
            }
            CoreError::Database(e.to_string())
        })?;
        Ok(())
    }

    async fn remove_owner(
        &self,
        registry: &str,
        package: &str,
        principal_type: &str,
        principal_id: &str,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "DELETE FROM package_owners \
             WHERE registry = $1 AND package_name = $2 \
               AND principal_type = $3 AND principal_id = $4",
        )
        .bind(registry)
        .bind(package)
        .bind(principal_type)
        .bind(principal_id)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn list_owners(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Vec<OwnerEntry>, CoreError> {
        let rows = sqlx::query(
            "SELECT principal_type, principal_id, role, granted_by \
             FROM package_owners \
             WHERE registry = $1 AND package_name = $2 \
             ORDER BY granted_at ASC",
        )
        .bind(registry)
        .bind(package)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows
            .into_iter()
            .map(|r| OwnerEntry {
                principal_type: r.get("principal_type"),
                principal_id: r.get("principal_id"),
                role: r.get("role"),
                granted_by: r.get("granted_by"),
            })
            .collect())
    }
}
