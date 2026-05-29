use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::{
    entities::{NamespacePackage, TeamNamespace, Visibility},
    error::CoreError,
    ports::TeamNamespacePort,
};

/// In-memory [`TeamNamespacePort`].
///
/// Stores namespace claims and per-package visibility overrides in separate maps.
///
/// `list_packages_in_namespace` always returns an empty list because querying
/// published packages requires access to the local-registry store, which is a
/// separate port. Use the PostgreSQL implementation for full functionality.
#[derive(Debug, Default)]
pub struct InMemoryTeamNamespaceStore {
    /// (registry, prefix) → TeamNamespace
    namespaces: Arc<RwLock<HashMap<(String, String), TeamNamespace>>>,
    /// (registry, package_name) → Visibility
    visibility: Arc<RwLock<HashMap<(String, String), Visibility>>>,
}

impl InMemoryTeamNamespaceStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

#[async_trait]
impl TeamNamespacePort for InMemoryTeamNamespaceStore {
    async fn find_namespace(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Option<TeamNamespace>, CoreError> {
        let map = self.namespaces.read().await;
        let best = map
            .iter()
            .filter(|((reg, prefix), _)| {
                reg == registry && (package == prefix || package.starts_with(&format!("{prefix}/")))
            })
            .max_by_key(|((_, prefix), _)| prefix.len());
        Ok(best.map(|(_, ns)| ns.clone()))
    }

    async fn list_namespaces(&self, registry: &str) -> Result<Vec<TeamNamespace>, CoreError> {
        let map = self.namespaces.read().await;
        let mut result: Vec<TeamNamespace> = map
            .iter()
            .filter(|((reg, _), _)| reg == registry)
            .map(|(_, ns)| ns.clone())
            .collect();
        result.sort_by(|a, b| a.prefix.cmp(&b.prefix));
        Ok(result)
    }

    async fn claim_namespace(&self, ns: TeamNamespace) -> Result<(), CoreError> {
        let mut map = self.namespaces.write().await;
        let key = (ns.registry.clone(), ns.prefix.clone());
        if map.contains_key(&key) {
            return Err(CoreError::Conflict(format!(
                "namespace '{}' in registry '{}' is already claimed",
                ns.prefix, ns.registry
            )));
        }
        map.insert(key, ns);
        Ok(())
    }

    async fn release_namespace(&self, registry: &str, prefix: &str) -> Result<(), CoreError> {
        let mut map = self.namespaces.write().await;
        map.remove(&(registry.to_owned(), prefix.to_owned()));
        Ok(())
    }

    async fn set_visibility(
        &self,
        registry: &str,
        package: &str,
        vis: Visibility,
    ) -> Result<(), CoreError> {
        let mut map = self.visibility.write().await;
        map.insert((registry.to_owned(), package.to_owned()), vis);
        Ok(())
    }

    async fn get_visibility(&self, registry: &str, package: &str) -> Result<Visibility, CoreError> {
        let map = self.visibility.read().await;
        Ok(map
            .get(&(registry.to_owned(), package.to_owned()))
            .cloned()
            .unwrap_or(Visibility::Public))
    }

    async fn list_namespaces_for_groups(
        &self,
        groups: &[String],
    ) -> Result<Vec<TeamNamespace>, CoreError> {
        let map = self.namespaces.read().await;
        let mut result: Vec<TeamNamespace> = map
            .values()
            .filter(|ns| groups.contains(&ns.group_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.registry.cmp(&b.registry).then(a.prefix.cmp(&b.prefix)));
        Ok(result)
    }

    /// Always returns an empty list.
    ///
    /// A full implementation requires querying the local-registry store, which
    /// is a separate port. Use [`PostgresLocalRegistry`] + [`PgTeamNamespaceStore`]
    /// together for this functionality.
    async fn list_packages_in_namespace(
        &self,
        _registry: &str,
        _prefix: &str,
        _limit: u64,
        _offset: u64,
    ) -> Result<Vec<NamespacePackage>, CoreError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use batlehub_core::{
        entities::{TeamNamespace, Visibility},
        ports::TeamNamespacePort,
    };

    use super::InMemoryTeamNamespaceStore;

    fn ns(registry: &str, prefix: &str, group: &str) -> TeamNamespace {
        TeamNamespace {
            registry: registry.to_owned(),
            prefix: prefix.to_owned(),
            group_id: group.to_owned(),
            claimed_by: None,
        }
    }

    #[tokio::test]
    async fn claim_and_find_namespace() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "frontend", "fe-team"))
            .await
            .unwrap();

        let found = store
            .find_namespace("reg", "frontend/my-lib")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().prefix, "frontend");
    }

    #[tokio::test]
    async fn find_namespace_exact_match() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "tools", "tools-team"))
            .await
            .unwrap();
        let found = store.find_namespace("reg", "tools").await.unwrap();
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn find_namespace_no_match() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "frontend", "fe-team"))
            .await
            .unwrap();
        let found = store
            .find_namespace("reg", "backend/service")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_namespace_longest_prefix_wins() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "a", "team-a"))
            .await
            .unwrap();
        store
            .claim_namespace(ns("reg", "a/b", "team-b"))
            .await
            .unwrap();
        let found = store.find_namespace("reg", "a/b/c").await.unwrap().unwrap();
        assert_eq!(found.prefix, "a/b");
    }

    #[tokio::test]
    async fn claim_conflict() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "frontend", "fe-team"))
            .await
            .unwrap();
        let result = store.claim_namespace(ns("reg", "frontend", "other")).await;
        assert!(matches!(
            result,
            Err(batlehub_core::error::CoreError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn release_namespace_silent_on_missing() {
        let store = InMemoryTeamNamespaceStore::new();
        store.release_namespace("reg", "nope").await.unwrap();
    }

    #[tokio::test]
    async fn release_then_not_found() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "frontend", "fe-team"))
            .await
            .unwrap();
        store.release_namespace("reg", "frontend").await.unwrap();
        assert!(store
            .find_namespace("reg", "frontend/x")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn set_and_get_visibility() {
        let store = InMemoryTeamNamespaceStore::new();
        assert_eq!(
            store.get_visibility("reg", "pkg").await.unwrap(),
            Visibility::Public
        );
        store
            .set_visibility("reg", "pkg", Visibility::Internal)
            .await
            .unwrap();
        assert_eq!(
            store.get_visibility("reg", "pkg").await.unwrap(),
            Visibility::Internal
        );
    }

    #[tokio::test]
    async fn list_namespaces_sorted_by_prefix() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "z-pkg", "t1"))
            .await
            .unwrap();
        store
            .claim_namespace(ns("reg", "a-pkg", "t2"))
            .await
            .unwrap();
        let list = store.list_namespaces("reg").await.unwrap();
        assert_eq!(list[0].prefix, "a-pkg");
        assert_eq!(list[1].prefix, "z-pkg");
    }

    #[tokio::test]
    async fn list_namespaces_for_groups() {
        let store = InMemoryTeamNamespaceStore::new();
        store
            .claim_namespace(ns("reg", "frontend", "fe-team"))
            .await
            .unwrap();
        store
            .claim_namespace(ns("reg", "backend", "be-team"))
            .await
            .unwrap();
        store
            .claim_namespace(ns("reg2", "infra", "fe-team"))
            .await
            .unwrap();

        let result = store
            .list_namespaces_for_groups(&["fe-team".to_owned()])
            .await
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|ns| ns.group_id == "fe-team"));
    }
}
