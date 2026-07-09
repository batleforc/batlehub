use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::{
    entities::{NamespacePackage, TeamNamespace, Visibility},
    error::CoreError,
    ports::{LocalRegistryBackend, TeamNamespacePort},
};

/// In-memory [`TeamNamespacePort`].
///
/// Stores namespace claims and per-package visibility overrides in separate maps.
///
/// `list_packages_in_namespace`/`count_packages_in_namespace` query `backend`
/// (when set via [`Self::with_backend`]) for published packages matching a
/// namespace prefix; without a backend they return empty, matching the
/// PostgreSQL implementation's behaviour when no local-registry store is wired.
#[derive(Default)]
pub struct InMemoryTeamNamespaceStore {
    /// (registry, prefix) → TeamNamespace
    namespaces: Arc<RwLock<HashMap<(String, String), TeamNamespace>>>,
    /// (registry, package_name) → Visibility
    visibility: Arc<RwLock<HashMap<(String, String), Visibility>>>,
    backend: Option<Arc<dyn LocalRegistryBackend>>,
}

impl std::fmt::Debug for InMemoryTeamNamespaceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryTeamNamespaceStore")
            .field("has_backend", &self.backend.is_some())
            .finish_non_exhaustive()
    }
}

impl InMemoryTeamNamespaceStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Wire a [`LocalRegistryBackend`] so `list_packages_in_namespace` and
    /// `count_packages_in_namespace` can query real published packages instead
    /// of always returning empty.
    pub fn with_backend(backend: Arc<dyn LocalRegistryBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend: Some(backend),
            ..Self::default()
        })
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

    /// Queries `backend` for published packages whose name matches `prefix`
    /// exactly or as a `prefix/...` path segment. Returns an empty list when
    /// no backend was wired via [`Self::with_backend`].
    async fn list_packages_in_namespace(
        &self,
        registry: &str,
        prefix: &str,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<NamespacePackage>, CoreError> {
        let Some(backend) = &self.backend else {
            return Ok(vec![]);
        };
        let all_names = backend.list_package_names(registry).await?;
        let mut matching: Vec<NamespacePackage> = vec![];
        for name in all_names {
            if !namespace_matches(&name, prefix) {
                continue;
            }
            let versions = backend.get_versions(registry, &name).await?;
            let vis = self.get_visibility(registry, &name).await?;
            for pkg in versions {
                matching.push(NamespacePackage {
                    name: pkg.name,
                    version: pkg.version,
                    visibility: vis.clone(),
                    published_by: pkg.published_by.unwrap_or_default(),
                    published_at: pkg.published_at,
                    yanked: pkg.yanked,
                });
            }
        }
        matching.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
        let start = offset as usize;
        let end = (offset + limit) as usize;
        Ok(matching
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect())
    }

    /// Counts published versions across packages matching `prefix`, via the
    /// same backend query as [`Self::list_packages_in_namespace`].
    async fn count_packages_in_namespace(
        &self,
        registry: &str,
        prefix: &str,
    ) -> Result<u64, CoreError> {
        let Some(backend) = &self.backend else {
            return Ok(0);
        };
        let all_names = backend.list_package_names(registry).await?;
        let mut total = 0u64;
        for name in all_names {
            if !namespace_matches(&name, prefix) {
                continue;
            }
            total += backend.get_versions(registry, &name).await?.len() as u64;
        }
        Ok(total)
    }
}

/// A package `name` belongs to namespace `prefix` if it equals the prefix
/// exactly or starts with `"{prefix}/"`.
fn namespace_matches(name: &str, prefix: &str) -> bool {
    name == prefix || (name.len() > prefix.len() && name.starts_with(&format!("{prefix}/")))
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
