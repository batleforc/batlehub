//! In-memory [`GrantRepository`].
//!
//! The behavioural reference the Postgres adapter is checked against. Where the
//! two could differ they are documented here, because a store that agrees with
//! its test double and not with production is the shape survey finding 2 came
//! in on — an empty accessible-registry list read as *every* registry in four
//! repository implementations "that all agreed with each other".

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::entities::SubjectMatcher;
use batlehub_core::error::CoreError;
use batlehub_core::ports::{version_node_key, GrantRepository, NodeKind, StoredGrant};

/// Keyed exactly as the unique constraint is: one row per subject per node.
type Key = (String, NodeKind, String, String);

#[derive(Default)]
pub struct InMemoryGrantRepository {
    rows: RwLock<HashMap<Key, StoredGrant>>,
}

impl InMemoryGrantRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn key(registry: &str, kind: NodeKind, node_key: &str, subject: &SubjectMatcher) -> Key {
        (
            registry.to_owned(),
            kind,
            node_key.to_owned(),
            subject.as_string(),
        )
    }
}

#[async_trait]
impl GrantRepository for InMemoryGrantRepository {
    async fn grants_for(
        &self,
        registry: &str,
        package: &str,
        version: Option<&str>,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        // An empty package matches no node. Returning every row for the registry
        // would be the vacuous-predicate shape finding 2 shipped: a query that
        // ran, matched everything, and looked like scoping.
        if package.is_empty() {
            return Ok(Vec::new());
        }
        let version_key = version.map(|v| version_node_key(package, v));
        let rows = self.rows.read().await;
        let mut out: Vec<StoredGrant> = rows
            .values()
            .filter(|g| {
                g.registry == registry
                    && match g.node_kind {
                        NodeKind::Package => g.node_key == package,
                        NodeKind::Version => version_key.as_deref() == Some(g.node_key.as_str()),
                    }
            })
            .cloned()
            .collect();
        // Package tier before version tier, then by subject: resolution unions
        // and so does not care, but `explain`'s provenance reports the *first*
        // node that granted a verb, and an unstable order would make that answer
        // change between identical requests.
        out.sort_by(|a, b| {
            (a.node_kind.as_str(), a.subject.as_string())
                .cmp(&(b.node_kind.as_str(), b.subject.as_string()))
        });
        Ok(out)
    }

    async fn put_grant(&self, grant: StoredGrant) -> Result<(), CoreError> {
        if grant.actions.is_empty() {
            // The schema's `cardinality(actions) > 0`, enforced here too: an
            // empty action set is what a *seal* is, and §4.3 confines sealing to
            // the config file. Accepting one would make the construct that takes
            // access away writable by a delegate.
            return Err(CoreError::InvalidInput(
                "a grant with no permissions is a seal, and seals are a config-file \
                 construct only (RFC 0015 §4.3)"
                    .to_owned(),
            ));
        }
        let key = Self::key(
            &grant.registry,
            grant.node_kind,
            &grant.node_key,
            &grant.subject,
        );
        self.rows.write().await.insert(key, grant);
        Ok(())
    }

    async fn delete_grant(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
        subject: &SubjectMatcher,
    ) -> Result<(), CoreError> {
        self.rows
            .write()
            .await
            .remove(&Self::key(registry, node_kind, node_key, subject));
        Ok(())
    }

    async fn package_grants_in_registry(
        &self,
        registry: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        let rows = self.rows.read().await;
        let mut out: Vec<StoredGrant> = rows
            .values()
            .filter(|g| g.registry == registry && g.node_kind == NodeKind::Package)
            .cloned()
            .collect();
        out.sort_by(|a, b| {
            (&a.node_key, a.subject.as_string()).cmp(&(&b.node_key, b.subject.as_string()))
        });
        Ok(out)
    }

    async fn grants_on_node(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        let rows = self.rows.read().await;
        let mut out: Vec<StoredGrant> = rows
            .values()
            .filter(|g| {
                g.registry == registry && g.node_kind == node_kind && g.node_key == node_key
            })
            .cloned()
            .collect();
        out.sort_by_key(|g| g.subject.as_string());
        Ok(out)
    }

    async fn version_grants_in_registry(
        &self,
        registry: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        Ok(self
            .rows
            .read()
            .await
            .values()
            .filter(|g| g.registry == registry && g.node_kind == NodeKind::Version)
            .cloned()
            .collect())
    }

    /// Matched by `package@…` for the same reason the delete below is: a bare
    /// prefix would take `@acme/billing-internal`'s rows for `@acme/billing`'s,
    /// which here would show a caller versions of a package they hold no grant
    /// on rather than deleting the wrong rows.
    async fn version_grants_for_package(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Vec<StoredGrant>, CoreError> {
        if package.is_empty() {
            return Ok(Vec::new());
        }
        let version_prefix = format!("{package}@");
        Ok(self
            .rows
            .read()
            .await
            .values()
            .filter(|g| {
                g.registry == registry
                    && g.node_kind == NodeKind::Version
                    && g.node_key.starts_with(&version_prefix)
            })
            .cloned()
            .collect())
    }

    async fn delete_package_grants(&self, registry: &str, package: &str) -> Result<(), CoreError> {
        if package.is_empty() {
            return Ok(());
        }
        // The version tier is matched by `package@…`, not by a bare prefix: a
        // prefix would take `@acme/billing-internal`'s rows out with
        // `@acme/billing`'s, which is the segment-boundary bug RFC 0011-bis §4.2
        // records, arriving on the delete path where it destroys rather than
        // discloses.
        let version_prefix = format!("{package}@");
        self.rows.write().await.retain(|_, g| {
            !(g.registry == registry
                && match g.node_kind {
                    NodeKind::Package => g.node_key == package,
                    NodeKind::Version => g.node_key.starts_with(&version_prefix),
                })
        });
        Ok(())
    }
}
