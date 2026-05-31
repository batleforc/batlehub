use std::str::FromStr;
use crate::db::DbResultExt;

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::{
    entities::{NamespacePackage, TeamNamespace, Visibility},
    error::CoreError,
    ports::TeamNamespacePort,
};

pub struct PgTeamNamespaceStore {
    pool: PgPool,
}

impl PgTeamNamespaceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl TeamNamespacePort for PgTeamNamespaceStore {
    async fn find_namespace(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Option<TeamNamespace>, CoreError> {
        // Longest-prefix match: a claim with prefix P covers package N when
        // N == P  OR  N starts with P + '/'.
        let row = sqlx::query(
            "SELECT prefix, group_id, claimed_by FROM team_namespaces \
             WHERE registry = $1 \
               AND ($2 = prefix \
                    OR (LENGTH($2) > LENGTH(prefix) \
                        AND SUBSTRING($2, 1, LENGTH(prefix) + 1) = prefix || '/')) \
             ORDER BY LENGTH(prefix) DESC \
             LIMIT 1",
        )
        .bind(registry)
        .bind(package)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        Ok(row.map(|r| TeamNamespace {
            registry: registry.to_owned(),
            prefix: r.get("prefix"),
            group_id: r.get("group_id"),
            claimed_by: r.get("claimed_by"),
        }))
    }

    async fn list_namespaces(&self, registry: &str) -> Result<Vec<TeamNamespace>, CoreError> {
        let rows = sqlx::query(
            "SELECT prefix, group_id, claimed_by FROM team_namespaces \
             WHERE registry = $1 \
             ORDER BY prefix ASC",
        )
        .bind(registry)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows
            .into_iter()
            .map(|r| TeamNamespace {
                registry: registry.to_owned(),
                prefix: r.get("prefix"),
                group_id: r.get("group_id"),
                claimed_by: r.get("claimed_by"),
            })
            .collect())
    }

    async fn claim_namespace(&self, ns: TeamNamespace) -> Result<(), CoreError> {
        sqlx::query(
            "INSERT INTO team_namespaces (registry, prefix, group_id, claimed_by) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&ns.registry)
        .bind(&ns.prefix)
        .bind(&ns.group_id)
        .bind(&ns.claimed_by)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db) = e {
                if db.constraint() == Some("uq_team_namespace") {
                    return CoreError::Conflict(format!(
                        "namespace '{}' in registry '{}' is already claimed",
                        ns.prefix, ns.registry
                    ));
                }
            }
            CoreError::Database(e.to_string())
        })?;
        Ok(())
    }

    async fn release_namespace(&self, registry: &str, prefix: &str) -> Result<(), CoreError> {
        sqlx::query("DELETE FROM team_namespaces WHERE registry = $1 AND prefix = $2")
            .bind(registry)
            .bind(prefix)
            .execute(&self.pool)
            .await
            .db_err()?;
        Ok(())
    }

    async fn set_visibility(
        &self,
        registry: &str,
        package: &str,
        vis: Visibility,
    ) -> Result<(), CoreError> {
        let result = sqlx::query(
            "UPDATE local_packages SET visibility = $3 \
             WHERE registry = $1 AND name = $2",
        )
        .bind(registry)
        .bind(package)
        .bind(vis.to_string())
        .execute(&self.pool)
        .await
        .db_err()?;

        if result.rows_affected() == 0 {
            return Err(CoreError::NotFound(format!(
                "package '{}' not found in registry '{}'",
                package, registry
            )));
        }
        Ok(())
    }

    async fn get_visibility(&self, registry: &str, package: &str) -> Result<Visibility, CoreError> {
        let row = sqlx::query(
            "SELECT visibility FROM local_packages \
             WHERE registry = $1 AND name = $2 AND status = 'published' \
             LIMIT 1",
        )
        .bind(registry)
        .bind(package)
        .fetch_optional(&self.pool)
        .await
        .db_err()?;

        match row {
            None => Ok(Visibility::Public),
            Some(r) => {
                let s: String = r.get("visibility");
                Visibility::from_str(&s)
                    .map_err(|e| CoreError::Database(format!("invalid visibility in db: {e}")))
            }
        }
    }

    async fn list_namespaces_for_groups(
        &self,
        groups: &[String],
    ) -> Result<Vec<TeamNamespace>, CoreError> {
        if groups.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query(
            "SELECT registry, prefix, group_id, claimed_by FROM team_namespaces \
             WHERE group_id = ANY($1) \
             ORDER BY registry, prefix ASC",
        )
        .bind(groups)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        Ok(rows
            .into_iter()
            .map(|r| TeamNamespace {
                registry: r.get("registry"),
                prefix: r.get("prefix"),
                group_id: r.get("group_id"),
                claimed_by: r.get("claimed_by"),
            })
            .collect())
    }

    async fn list_packages_in_namespace(
        &self,
        registry: &str,
        prefix: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<NamespacePackage>, CoreError> {
        let rows = sqlx::query(
            "SELECT name, version, visibility, published_by, published_at, yanked \
             FROM local_packages \
             WHERE registry = $1 \
               AND status = 'published' \
               AND (name = $2 \
                    OR (LENGTH(name) > LENGTH($2) \
                        AND SUBSTRING(name, 1, LENGTH($2) + 1) = $2 || '/')) \
             ORDER BY name, version \
             LIMIT $3 OFFSET $4",
        )
        .bind(registry)
        .bind(prefix)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        rows.into_iter()
            .map(|r| {
                let vis_str: String = r.get("visibility");
                let vis = Visibility::from_str(&vis_str)
                    .map_err(|e| CoreError::Database(format!("invalid visibility in db: {e}")))?;
                Ok(NamespacePackage {
                    name: r.get("name"),
                    version: r.get("version"),
                    visibility: vis,
                    published_by: r.get("published_by"),
                    published_at: r.get("published_at"),
                    yanked: r.get("yanked"),
                })
            })
            .collect()
    }
}
