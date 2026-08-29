//! In-memory [`PolicyRepository`].
//!
//! The behavioural reference the Postgres adapter is checked against, for the
//! same reason its `grants` sibling is one: a store that agrees with its test
//! double and not with production is the shape survey finding 2 came in on, four
//! repository implementations "that all agreed with each other". Where the two
//! could differ it is documented here.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::error::CoreError;
use batlehub_core::ports::{version_node_key, NodeKind, PolicyRepository, StoredPolicy};

/// Keyed exactly as the unique constraint is: **one row per node**.
///
/// One fewer component than the grant store's key, and the difference is the
/// model's: repeating a subject on a grant node is a union, so grants are keyed
/// by subject too. A node has exactly one policy, and a second row would be a
/// second answer to "what applies here" with no rule for choosing between them.
type Key = (String, NodeKind, String);

#[derive(Default)]
pub struct InMemoryPolicyRepository {
    rows: RwLock<HashMap<Key, StoredPolicy>>,
}

impl InMemoryPolicyRepository {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn key(registry: &str, kind: NodeKind, node_key: &str) -> Key {
        (registry.to_owned(), kind, node_key.to_owned())
    }
}

#[async_trait]
impl PolicyRepository for InMemoryPolicyRepository {
    async fn policy_for(
        &self,
        registry: &str,
        package: &str,
        version: Option<&str>,
    ) -> Result<Vec<StoredPolicy>, CoreError> {
        // An empty package matches no node. Returning every row for the registry
        // would be the vacuous-predicate shape finding 2 shipped: a query that
        // ran, matched everything, and looked like scoping.
        if package.is_empty() {
            return Ok(Vec::new());
        }
        let version_key = version.map(|v| version_node_key(package, v));
        let rows = self.rows.read().await;
        let mut out: Vec<StoredPolicy> = rows
            .values()
            .filter(|p| {
                p.registry == registry
                    && match p.node_kind {
                        NodeKind::Package => p.node_key == package,
                        NodeKind::Version => version_key.as_deref() == Some(p.node_key.as_str()),
                    }
            })
            .cloned()
            .collect();
        // **Deepest last**, which the port promises and composition depends on:
        // `PolicyPath::resolve` walks in order and takes the last declaration,
        // so a package row arriving after its version row would let the shallower
        // tier win. Unlike the grant store's ordering — where resolution unions
        // and the order only affects `explain`'s provenance — this ordering is
        // load-bearing for the answer itself.
        out.sort_by_key(|p| match p.node_kind {
            NodeKind::Package => 0,
            NodeKind::Version => 1,
        });
        Ok(out)
    }

    async fn put_policy(&self, policy: StoredPolicy) -> Result<(), CoreError> {
        if let Some(reason) = policy.validate() {
            return Err(CoreError::InvalidInput(reason));
        }
        let key = Self::key(&policy.registry, policy.node_kind, &policy.node_key);
        if policy.is_empty() {
            // A row declaring nothing is not a policy. Storing one would make
            // "has a policy node" and "has a policy" different questions, and a
            // resolver that found the node would compose an override that
            // overrides nothing — which reads, in `explain`, as a tier the
            // operator set and cannot find.
            self.rows.write().await.remove(&key);
            return Ok(());
        }
        self.rows.write().await.insert(key, policy);
        Ok(())
    }

    async fn delete_policy(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<(), CoreError> {
        self.rows
            .write()
            .await
            .remove(&Self::key(registry, node_kind, node_key));
        Ok(())
    }

    async fn policy_on_node(
        &self,
        registry: &str,
        node_kind: NodeKind,
        node_key: &str,
    ) -> Result<Option<StoredPolicy>, CoreError> {
        Ok(self
            .rows
            .read()
            .await
            .get(&Self::key(registry, node_kind, node_key))
            .cloned())
    }

    async fn exemptions_in_registry(&self, registry: &str) -> Result<Vec<StoredPolicy>, CoreError> {
        let rows = self.rows.read().await;
        let mut out: Vec<StoredPolicy> = rows
            .values()
            .filter(|p| {
                p.registry == registry
                    && p.node_kind == NodeKind::Version
                    // An `exempt: true` flag, not merely a rule override: a
                    // namespace re-tuning `cve_gate`'s threshold writes an entry
                    // under the same gate name and is not an exemption.
                    && p.rules.iter().any(|r| {
                        r.settings
                            .get("exempt")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    })
            })
            .cloned()
            .collect();
        out.sort_by(|a, b| a.node_key.cmp(&b.node_key));
        Ok(out)
    }

    async fn delete_package_policy(&self, registry: &str, package: &str) -> Result<(), CoreError> {
        if package.is_empty() {
            return Ok(());
        }
        // `package@`, not a bare prefix: a prefix would take
        // `@acme/billing-internal`'s rows out with `@acme/billing`'s, which is
        // RFC 0011-bis §4.2's segment-boundary bug arriving on the delete path,
        // where it destroys rather than discloses.
        let version_prefix = format!("{package}@");
        self.rows.write().await.retain(|_, p| {
            !(p.registry == registry
                && match p.node_kind {
                    NodeKind::Package => p.node_key == package,
                    NodeKind::Version => p.node_key.starts_with(&version_prefix),
                })
        });
        Ok(())
    }
}
