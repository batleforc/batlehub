//! Postgres [`GrantRepository`] — RFC 0015 §6.3's `grants` table.
//!
//! Checked against `InMemoryGrantRepository`, which is the behavioural
//! reference. The two must agree on the edges as well as the middle: survey
//! finding 2 shipped because an empty list meant "everything" in four repository
//! implementations "that all agreed with each other", so agreement alone is not
//! evidence — `crates/adapters/tests/pg_grants.rs` asserts the same properties
//! against both.

use async_trait::async_trait;
use sqlx::{PgPool, Row};

use batlehub_core::entities::{Action, SubjectMatcher};
use batlehub_core::error::CoreError;
use batlehub_core::ports::{version_node_key, GrantRepository, NodeKind, StoredGrant};

use crate::db::DbResultExt;

pub struct PgGrantRepository {
    pool: PgPool,
}

impl PgGrantRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// One row → a [`StoredGrant`].
///
/// A row whose `subject` or `actions` this build cannot parse is an **error**,
/// not a skip. Skipping would silently drop a grant an operator wrote and can
/// see in the table — the "granted to nobody, noticed by nobody" failure the
/// closed `Action` enum exists to remove, arriving through storage instead of
/// through config.
fn row_to_grant(row: &sqlx::postgres::PgRow) -> Result<StoredGrant, CoreError> {
    let node_kind: String = row.try_get("node_kind").db_err()?;
    let subject: String = row.try_get("subject").db_err()?;
    let actions: Vec<String> = row.try_get("actions").db_err()?;

    Ok(StoredGrant {
        registry: row.try_get("registry").db_err()?,
        node_kind: node_kind.parse()?,
        node_key: row.try_get("node_key").db_err()?,
        subject: SubjectMatcher::parse(&subject)
            .map_err(|e| CoreError::InvalidInput(format!("stored grant subject: {e}")))?,
        actions: actions
            .iter()
            .map(|a| {
                a.parse::<Action>()
                    .map_err(|e| CoreError::InvalidInput(format!("stored grant action: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?,
        granted_by: row.try_get("granted_by").db_err()?,
    })
}

#[async_trait]
impl GrantRepository for PgGrantRepository {
    async fn grants_for(
        &self,
        registry: &str,
        package: &str,
        version: Option<&str>,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        // An empty package names no node. Without this the `node_key = ''`
        // comparison below is simply false and the answer is the same — but the
        // guard is here because the *next* edit is the one that makes it
        // vacuous, and finding 2 is what a vacuous predicate costs.
        if package.is_empty() {
            return Ok(Vec::new());
        }
        // Both tiers in one round trip: §11.7 budgets 2 ms p99 for a
        // single-coordinate `authorize`, and two queries would spend it twice.
        //
        // `node_key = ANY($2)` rather than an OR of equality and a LIKE: the
        // version key is exact, and a prefix match would take
        // `@acme/billing-internal@1.0.0` for `@acme/billing`'s — the
        // segment-boundary bug RFC 0011-bis §4.2 records, on the read path.
        let mut keys = vec![package.to_owned()];
        if let Some(v) = version {
            keys.push(version_node_key(package, v));
        }
        // Written out rather than composed: `sqlx::query` takes a `&'static str`
        // on purpose, and the one place a column list is worth sharing is the
        // one place a dynamic string could become an injection.
        let rows = sqlx::query(
            "SELECT registry, node_kind, node_key, subject, actions, granted_by \
             FROM grants \
             WHERE registry = $1 AND node_key = ANY($2) \
             ORDER BY node_kind, subject",
        )
        .bind(registry)
        .bind(&keys)
        .fetch_all(&self.pool)
        .await
        .db_err()?;

        rows.iter().map(row_to_grant).collect()
    }

    async fn put_grant(&self, grant: StoredGrant) -> Result<(), CoreError> {
        if grant.actions.is_empty() {
            // Mirrors `ck_grants_actions_non_empty`, so the message names the
            // model rather than the constraint. An empty action set is what a
            // seal *is*, and §4.3 confines sealing to the config file.
            return Err(CoreError::InvalidInput(
                "a grant with no permissions is a seal, and seals are a config-file \
                 construct only (RFC 0015 §4.3)"
                    .to_owned(),
            ));
        }
        let actions: Vec<String> = grant.actions.iter().map(|a| a.to_string()).collect();
        sqlx::query(
            "INSERT INTO grants (registry, node_kind, node_key, subject, actions, granted_by) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (registry, node_kind, node_key, subject) \
             DO UPDATE SET actions = EXCLUDED.actions, \
                           granted_by = EXCLUDED.granted_by, \
                           granted_at = NOW()",
        )
        .bind(&grant.registry)
        .bind(grant.node_kind.as_str())
        .bind(&grant.node_key)
        .bind(grant.subject.as_string())
        .bind(&actions)
        .bind(&grant.granted_by)
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn delete_grant(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
        subject: &SubjectMatcher,
    ) -> Result<(), CoreError> {
        sqlx::query(
            "DELETE FROM grants \
             WHERE registry = $1 AND node_kind = $2 AND node_key = $3 AND subject = $4",
        )
        .bind(registry)
        .bind(node_kind.as_str())
        .bind(node_key)
        .bind(subject.as_string())
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }

    async fn package_grants_in_registry(
        &self,
        registry: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        let rows = sqlx::query(
            "SELECT registry, node_kind, node_key, subject, actions, granted_by \
             FROM grants \
             WHERE registry = $1 AND node_kind = 'package' \
             ORDER BY node_key, subject",
        )
        .bind(registry)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        rows.iter().map(row_to_grant).collect()
    }

    async fn grants_on_node(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        let rows = sqlx::query(
            "SELECT registry, node_kind, node_key, subject, actions, granted_by \
             FROM grants \
             WHERE registry = $1 AND node_kind = $2 AND node_key = $3 \
             ORDER BY subject",
        )
        .bind(registry)
        .bind(node_kind.as_str())
        .bind(node_key)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        rows.iter().map(row_to_grant).collect()
    }

    async fn version_grants_in_registry(
        &self,
        registry: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        let rows = sqlx::query(
            "SELECT registry, node_kind, node_key, subject, actions, granted_by \
             FROM grants WHERE registry = $1 AND node_kind = 'version'",
        )
        .bind(registry)
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        rows.iter().map(row_to_grant).collect()
    }

    /// Same `LIKE` discipline as [`Self::delete_package_grants`] below, for the
    /// same reason: a package name may contain `%` or `_`, and an unescaped one
    /// would return another package's grants — which here would *widen* a
    /// listing rather than narrow it.
    async fn version_grants_for_package(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        if package.is_empty() {
            return Ok(Vec::new());
        }
        let escaped = package
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let rows = sqlx::query(
            "SELECT registry, node_kind, node_key, subject, actions, granted_by \
             FROM grants \
             WHERE registry = $1 AND node_kind = 'version' AND node_key LIKE $2 ESCAPE '\\'",
        )
        .bind(registry)
        .bind(format!("{escaped}@%"))
        .fetch_all(&self.pool)
        .await
        .db_err()?;
        rows.iter().map(row_to_grant).collect()
    }

    async fn delete_package_grants(&self, registry: &str, package: &str) -> Result<(), CoreError> {
        if package.is_empty() {
            return Ok(());
        }
        // `node_key LIKE $3` with the pattern characters escaped, not
        // interpolated: a package name may contain `%` or `_` — both are legal
        // in an npm name — and an unescaped one would delete grants for every
        // package that happens to match. The escape character is stated rather
        // than defaulted, which is the same discipline RFC 0011-bis §4.2 asks of
        // the namespace predicate.
        let escaped = package
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        sqlx::query(
            "DELETE FROM grants \
             WHERE registry = $1 \
               AND ( (node_kind = 'package' AND node_key = $2) \
                  OR (node_kind = 'version' AND node_key LIKE $3 ESCAPE '\\') )",
        )
        .bind(registry)
        .bind(package)
        .bind(format!("{escaped}@%"))
        .execute(&self.pool)
        .await
        .db_err()?;
        Ok(())
    }
}
