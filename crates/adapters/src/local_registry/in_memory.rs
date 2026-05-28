use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use batlehub_core::{entities::PublishedPackage, error::CoreError, ports::LocalRegistryBackend};

/// Record status, mirroring the `pending`/`published` lifecycle of
/// [`PostgresLocalRegistry`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordStatus {
    Pending,
    Published,
}

#[derive(Debug, Clone)]
struct Record {
    pkg: PublishedPackage,
    status: RecordStatus,
    inserted_at: DateTime<Utc>,
}

type PackageKey = String; // "{registry}:{name}"
type VersionKey = String; // version string

/// A fully spec-compliant in-memory [`LocalRegistryBackend`].
///
/// Implements the three-step publish protocol (`publish` → artifact write →
/// `commit_publish`), conflict detection on published versions,
/// `cleanup_pending`, and `list_package_names`.
///
/// Intended for integration tests, single-binary demos, and any context that
/// does not need persistence across process restarts. Thread-safe via
/// `tokio::sync::RwLock`.
#[derive(Debug, Default)]
pub struct InMemoryLocalRegistry {
    inner: Arc<RwLock<HashMap<PackageKey, HashMap<VersionKey, Record>>>>,
}

impl InMemoryLocalRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

fn pkg_key(registry: &str, name: &str) -> PackageKey {
    format!("{registry}:{name}")
}

#[async_trait]
impl LocalRegistryBackend for InMemoryLocalRegistry {
    /// Insert the version in *pending* state, invisible to `get_versions` /
    /// `exists` until `commit_publish` is called.
    ///
    /// Returns `CoreError::Conflict` if a *published* version already exists.
    /// Silently overwrites a stale *pending* row (crash recovery for callers
    /// that retry after a partial failure).
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        let versions = map.entry(pkg_key(&pkg.registry, &pkg.name)).or_default();

        if let Some(existing) = versions.get(&pkg.version) {
            if existing.status == RecordStatus::Published {
                return Err(CoreError::Conflict(format!(
                    "{}@{} already published in registry '{}'",
                    pkg.name, pkg.version, pkg.registry
                )));
            }
            // Stale pending row: fall through and overwrite below.
        }

        versions.insert(
            pkg.version.clone(),
            Record { pkg, status: RecordStatus::Pending, inserted_at: Utc::now() },
        );
        Ok(())
    }

    /// Promote the pending row to *published*. No-op if the row is missing.
    async fn commit_publish(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        if let Some(versions) = map.get_mut(&pkg_key(registry, name)) {
            if let Some(record) = versions.get_mut(version) {
                record.status = RecordStatus::Published;
            }
        }
        Ok(())
    }

    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        if let Some(versions) = map.get_mut(&pkg_key(registry, name)) {
            if let Some(r) = versions.get_mut(version) {
                if r.status == RecordStatus::Published {
                    r.pkg.yanked = true;
                    if let Some(obj) = r.pkg.index_metadata.as_object_mut() {
                        obj.insert("yanked".to_owned(), serde_json::Value::Bool(true));
                    }
                }
            }
        }
        Ok(())
    }

    async fn unyank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        if let Some(versions) = map.get_mut(&pkg_key(registry, name)) {
            if let Some(r) = versions.get_mut(version) {
                if r.status == RecordStatus::Published {
                    r.pkg.yanked = false;
                    if let Some(obj) = r.pkg.index_metadata.as_object_mut() {
                        obj.insert("yanked".to_owned(), serde_json::Value::Bool(false));
                    }
                }
            }
        }
        Ok(())
    }

    async fn get_versions(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        let map = self.inner.read().await;
        let mut result: Vec<PublishedPackage> = map
            .get(&pkg_key(registry, name))
            .map(|vs| {
                vs.values()
                    .filter(|r| r.status == RecordStatus::Published)
                    .map(|r| r.pkg.clone())
                    .collect()
            })
            .unwrap_or_default();
        result.sort_by_key(|p| p.published_at);
        Ok(result)
    }

    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
        let map = self.inner.read().await;
        Ok(map
            .get(&pkg_key(registry, name))
            .map(|vs| vs.values().any(|r| r.status == RecordStatus::Published))
            .unwrap_or(false))
    }

    async fn remove_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<(), CoreError> {
        let mut map = self.inner.write().await;
        if let Some(versions) = map.get_mut(&pkg_key(registry, name)) {
            versions.remove(version);
        }
        Ok(())
    }

    /// Remove *pending* rows whose `inserted_at` is older than `older_than`.
    /// Published rows are never touched. Returns the number of rows deleted.
    async fn cleanup_pending(&self, older_than: Duration) -> Result<u64, CoreError> {
        // chrono::Duration::from_std fails only on absurd durations (>292 years).
        // Treat that as "nothing qualifies" rather than wiping the entire pending set.
        let Ok(std_dur) = chrono::Duration::from_std(older_than) else {
            return Ok(0);
        };
        let cutoff = Utc::now() - std_dur;
        let mut map = self.inner.write().await;
        let mut removed = 0u64;
        for versions in map.values_mut() {
            let before = versions.len();
            versions
                .retain(|_, r| !(r.status == RecordStatus::Pending && r.inserted_at < cutoff));
            removed += (before - versions.len()) as u64;
        }
        Ok(removed)
    }

    /// Return distinct package names that have at least one *published* version
    /// in `registry`, sorted alphabetically.
    async fn list_package_names(&self, registry: &str) -> Result<Vec<String>, CoreError> {
        let prefix = format!("{registry}:");
        let mut names: Vec<String> = {
            let map = self.inner.read().await;
            map.iter()
                .filter(|(k, vs)| {
                    k.starts_with(&prefix)
                        && vs.values().any(|r| r.status == RecordStatus::Published)
                })
                .map(|(k, _)| k[prefix.len()..].to_owned())
                .collect()
        }; // read lock dropped here
        names.sort();
        Ok(names)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use batlehub_core::{entities::PublishedPackage, error::CoreError, ports::LocalRegistryBackend};

    use super::InMemoryLocalRegistry;

    fn pkg(registry: &str, name: &str, version: &str) -> PublishedPackage {
        PublishedPackage {
            registry: registry.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            checksum: format!("sha256-{version}"),
            yanked: false,
            index_metadata: serde_json::json!({"yanked": false}),
            published_at: Utc::now(),
            published_by: Some("test-user".to_owned()),
            signature_bytes: None,
            signature_type: None,
            visibility: Default::default(),
        }
    }

    /// Publish then commit makes the version visible.
    #[tokio::test]
    async fn commit_promotes_pending_to_published() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();

        // Pending — not visible yet.
        assert!(!store.exists("reg", "foo").await.unwrap());
        assert!(store.get_versions("reg", "foo").await.unwrap().is_empty());

        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();

        assert!(store.exists("reg", "foo").await.unwrap());
        let versions = store.get_versions("reg", "foo").await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "1.0.0");
    }

    /// Publishing a duplicate *published* version returns Conflict.
    #[tokio::test]
    async fn duplicate_published_version_is_conflict() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();

        let err = store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap_err();
        assert!(
            matches!(err, CoreError::Conflict(_)),
            "expected Conflict, got {err:?}"
        );
    }

    /// A stale pending row (from a prior crash) is silently overwritten so the
    /// caller can retry.
    #[tokio::test]
    async fn stale_pending_row_is_overwritten_on_retry() {
        let store = InMemoryLocalRegistry::new();
        // First attempt — crashes before commit.
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        // Retry — must succeed, not return Conflict.
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();
        assert!(store.exists("reg", "foo").await.unwrap());
    }

    /// Yank sets `yanked = true` and updates `index_metadata`.
    #[tokio::test]
    async fn yank_sets_flag_and_metadata() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();
        store.yank("reg", "foo", "1.0.0").await.unwrap();

        let versions = store.get_versions("reg", "foo").await.unwrap();
        assert!(versions[0].yanked);
        assert_eq!(versions[0].index_metadata["yanked"], serde_json::Value::Bool(true));
    }

    /// Unyank reverses a yank.
    #[tokio::test]
    async fn unyank_clears_flag_and_metadata() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();
        store.yank("reg", "foo", "1.0.0").await.unwrap();
        store.unyank("reg", "foo", "1.0.0").await.unwrap();

        let versions = store.get_versions("reg", "foo").await.unwrap();
        assert!(!versions[0].yanked);
        assert_eq!(versions[0].index_metadata["yanked"], serde_json::Value::Bool(false));
    }

    /// `remove_version` deletes a record regardless of its status.
    #[tokio::test]
    async fn remove_version_deletes_published_record() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();
        store.remove_version("reg", "foo", "1.0.0").await.unwrap();

        assert!(!store.exists("reg", "foo").await.unwrap());
        assert!(store.get_versions("reg", "foo").await.unwrap().is_empty());
    }

    /// `remove_version` on a pending row (rollback scenario).
    #[tokio::test]
    async fn remove_version_deletes_pending_record() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        // Do NOT commit — simulate rollback.
        store.remove_version("reg", "foo", "1.0.0").await.unwrap();

        // A fresh publish of the same version must now succeed.
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "foo", "1.0.0").await.unwrap();
        assert!(store.exists("reg", "foo").await.unwrap());
    }

    /// `cleanup_pending` removes old pending rows but never published ones.
    #[tokio::test]
    async fn cleanup_pending_removes_old_pending_only() {
        use std::ops::Sub;

        let store = InMemoryLocalRegistry::new();

        // Insert a published version — must survive cleanup.
        store.publish(pkg("reg", "bar", "1.0.0")).await.unwrap();
        store.commit_publish("reg", "bar", "1.0.0").await.unwrap();

        // Insert a fresh pending version — too new to be cleaned up.
        store.publish(pkg("reg", "bar", "2.0.0")).await.unwrap();

        // Manually backdate the pending row so it looks old.
        {
            let mut map = store.inner.write().await;
            let key = super::pkg_key("reg", "bar");
            if let Some(r) = map.get_mut(&key).and_then(|vs| vs.get_mut("2.0.0")) {
                r.inserted_at = Utc::now().sub(chrono::Duration::hours(2));
            }
        }

        let removed = store.cleanup_pending(Duration::from_secs(3600)).await.unwrap();
        assert_eq!(removed, 1, "expected 1 pending row removed");

        // Published 1.0.0 must still be visible.
        assert!(store.exists("reg", "bar").await.unwrap());
        let versions = store.get_versions("reg", "bar").await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "1.0.0");
    }

    /// `cleanup_pending` with zero duration removes nothing if all pending rows
    /// are brand-new.
    #[tokio::test]
    async fn cleanup_pending_leaves_fresh_pending_intact() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg", "foo", "1.0.0")).await.unwrap();
        let removed = store.cleanup_pending(Duration::from_secs(3600)).await.unwrap();
        assert_eq!(removed, 0);
    }

    /// `list_package_names` returns alphabetically sorted names of packages
    /// with at least one published version, excluding packages with only
    /// pending versions.
    #[tokio::test]
    async fn list_package_names_published_only_sorted() {
        let store = InMemoryLocalRegistry::new();

        for name in ["charlie", "alpha", "beta"] {
            store.publish(pkg("reg", name, "1.0.0")).await.unwrap();
            store.commit_publish("reg", name, "1.0.0").await.unwrap();
        }

        // Pending-only package — must not appear.
        store.publish(pkg("reg", "delta", "1.0.0")).await.unwrap();

        // Different registry — must not appear.
        store.publish(pkg("other-reg", "zeta", "1.0.0")).await.unwrap();
        store.commit_publish("other-reg", "zeta", "1.0.0").await.unwrap();

        let names = store.list_package_names("reg").await.unwrap();
        assert_eq!(names, vec!["alpha", "beta", "charlie"]);
    }

    /// `get_versions` returns results sorted by `published_at` ASC.
    #[tokio::test]
    async fn get_versions_sorted_by_published_at() {
        let store = InMemoryLocalRegistry::new();

        let t0 = Utc::now();
        let mut v1 = pkg("reg", "foo", "1.0.0");
        let mut v2 = pkg("reg", "foo", "2.0.0");
        let mut v3 = pkg("reg", "foo", "3.0.0");
        v1.published_at = t0;
        v2.published_at = t0 + chrono::Duration::seconds(1);
        v3.published_at = t0 + chrono::Duration::seconds(2);

        // Publish in reverse order to verify sort.
        for v in [v3.clone(), v1.clone(), v2.clone()] {
            let ver = v.version.clone();
            store.publish(v).await.unwrap();
            store.commit_publish("reg", "foo", &ver).await.unwrap();
        }

        let versions = store.get_versions("reg", "foo").await.unwrap();
        let got: Vec<&str> = versions.iter().map(|p| p.version.as_str()).collect();
        assert_eq!(got, vec!["1.0.0", "2.0.0", "3.0.0"]);
    }

    /// `exists` returns false for an unknown package.
    #[tokio::test]
    async fn exists_false_for_unknown_package() {
        let store = InMemoryLocalRegistry::new();
        assert!(!store.exists("reg", "unknown").await.unwrap());
    }

    /// The default `bulk_yank` implementation yanks multiple versions in one call.
    #[tokio::test]
    async fn bulk_yank_yanks_multiple_versions() {
        let store = InMemoryLocalRegistry::new();
        for v in ["1.0.0", "2.0.0", "3.0.0"] {
            store.publish(pkg("reg", "foo", v)).await.unwrap();
            store.commit_publish("reg", "foo", v).await.unwrap();
        }

        let result = store
            .bulk_yank("reg", &[("foo".to_owned(), "1.0.0".to_owned()), ("foo".to_owned(), "2.0.0".to_owned())])
            .await
            .unwrap();
        assert_eq!(result.succeeded, 2);
        assert!(result.failed.is_empty());

        let versions = store.get_versions("reg", "foo").await.unwrap();
        let yanked: Vec<&str> = versions.iter().filter(|p| p.yanked).map(|p| p.version.as_str()).collect();
        assert_eq!(yanked, vec!["1.0.0", "2.0.0"]);
        assert!(!versions.iter().find(|p| p.version == "3.0.0").unwrap().yanked);
    }

    /// `bulk_unyank` reverses a bulk yank.
    #[tokio::test]
    async fn bulk_unyank_unyanks_multiple_versions() {
        let store = InMemoryLocalRegistry::new();
        for v in ["1.0.0", "2.0.0"] {
            store.publish(pkg("reg", "foo", v)).await.unwrap();
            store.commit_publish("reg", "foo", v).await.unwrap();
            store.yank("reg", "foo", v).await.unwrap();
        }

        let result = store
            .bulk_unyank("reg", &[("foo".to_owned(), "1.0.0".to_owned()), ("foo".to_owned(), "2.0.0".to_owned())])
            .await
            .unwrap();
        assert_eq!(result.succeeded, 2);

        let versions = store.get_versions("reg", "foo").await.unwrap();
        assert!(versions.iter().all(|p| !p.yanked));
    }

    /// `bulk_remove_versions` permanently deletes multiple versions.
    #[tokio::test]
    async fn bulk_remove_deletes_multiple_versions() {
        let store = InMemoryLocalRegistry::new();
        for v in ["1.0.0", "2.0.0", "3.0.0"] {
            store.publish(pkg("reg", "foo", v)).await.unwrap();
            store.commit_publish("reg", "foo", v).await.unwrap();
        }

        let result = store
            .bulk_remove_versions("reg", &[("foo".to_owned(), "1.0.0".to_owned()), ("foo".to_owned(), "3.0.0".to_owned())])
            .await
            .unwrap();
        assert_eq!(result.succeeded, 2);

        let versions = store.get_versions("reg", "foo").await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, "2.0.0");
    }

    /// Operations on a different registry name are fully isolated.
    #[tokio::test]
    async fn registry_namespaces_are_isolated() {
        let store = InMemoryLocalRegistry::new();
        store.publish(pkg("reg-a", "foo", "1.0.0")).await.unwrap();
        store.commit_publish("reg-a", "foo", "1.0.0").await.unwrap();

        assert!(!store.exists("reg-b", "foo").await.unwrap());
        assert!(store.get_versions("reg-b", "foo").await.unwrap().is_empty());
    }
}
