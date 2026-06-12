use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;

use super::*;
use crate::{
    entities::{Identity, Role},
    error::CoreError,
    ports::{StorageBackend, StorageMeta, StoredArtifact},
    services::hot_config::{new_hot_lock, HotConfig},
};

// ── Minimal mock backend ──────────────────────────────────────────────────

#[derive(Default)]
struct InMemBackend {
    versions: Mutex<Vec<PublishedPackage>>,
}

impl InMemBackend {
    fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn seed(&self, pkg: PublishedPackage) {
        self.versions.lock().unwrap().push(pkg);
    }
}

#[async_trait]
impl crate::ports::LocalRegistryBackend for InMemBackend {
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
        self.versions.lock().unwrap().push(pkg);
        Ok(())
    }
    async fn yank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn unyank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn get_versions(
        &self,
        registry: &str,
        name: &str,
    ) -> Result<Vec<PublishedPackage>, CoreError> {
        Ok(self
            .versions
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.registry == registry && p.name == name)
            .cloned()
            .collect())
    }
    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
        Ok(self
            .versions
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.registry == registry && p.name == name))
    }
}

struct NoopStorage;

#[async_trait]
impl StorageBackend for NoopStorage {
    async fn store(&self, _: &str, _: Bytes, _: StorageMeta) -> Result<(), CoreError> {
        Ok(())
    }
    async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> {
        Ok(None)
    }
    async fn exists(&self, _: &str) -> Result<bool, CoreError> {
        Ok(false)
    }
    async fn delete(&self, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> {
        Ok(0)
    }
    async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> {
        Ok((0, 0))
    }
    async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> {
        Ok(vec![])
    }
}

fn svc(backend: Arc<InMemBackend>, max_bytes: Option<u64>) -> LocalRegistryService {
    LocalRegistryService {
        backend,
        storage: Arc::new(NoopStorage),
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: max_bytes,
        }),
        quota: None,
        ownership: None,
        team_namespace: None,
        sbom: None,
        explore_cache: None,
    }
}

fn pkg(registry: &str, name: &str, version: &str) -> PublishedPackage {
    PublishedPackage {
        registry: registry.to_owned(),
        name: name.to_owned(),
        version: version.to_owned(),
        checksum: "abc".to_owned(),
        yanked: false,
        index_metadata: serde_json::json!({}),
        published_at: Utc::now(),
        published_by: None,
        signature_bytes: None,
        signature_type: None,
        visibility: Default::default(),
    }
}

fn anon() -> Identity {
    Identity {
        user_id: None,
        role: Role::Anonymous,
        auth_provider: None,
        groups: vec![],
    }
}

fn user() -> Identity {
    Identity {
        user_id: Some("u1".into()),
        role: Role::User,
        auth_provider: None,
        groups: vec![],
    }
}

// ── publish error paths ───────────────────────────────────────────────────

#[tokio::test]
async fn publish_rejects_oversized_artifact() {
    let backend = InMemBackend::arc();
    let s = svc(backend, Some(10)); // 10-byte limit
    let req = PublishRequest {
        registry: "npm".into(),
        name: "big".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from(vec![0u8; 11]), // 11 bytes > 10-byte limit
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: user(),
        signature_bytes: None,
        signature_type: None,
    };
    let err = s.publish(req).await.unwrap_err();
    assert!(matches!(err, CoreError::PayloadTooLarge(_)));
}

#[tokio::test]
async fn publish_rejects_path_traversal_in_name() {
    let s = svc(InMemBackend::arc(), None);
    let req = PublishRequest {
        registry: "npm".into(),
        name: "../../../../etc/cron.d/evil".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from_static(b"payload"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: user(),
        signature_bytes: None,
        signature_type: None,
    };
    let err = s.publish(req).await.unwrap_err();
    assert!(
        matches!(err, CoreError::InvalidInput(_)),
        "traversal name must be rejected, got {err:?}"
    );
}

#[tokio::test]
async fn publish_rejects_path_traversal_in_version() {
    let s = svc(InMemBackend::arc(), None);
    let req = PublishRequest {
        registry: "npm".into(),
        name: "pkg".into(),
        version: "../../../../tmp/evil".into(),
        artifact: Bytes::from_static(b"payload"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: user(),
        signature_bytes: None,
        signature_type: None,
    };
    let err = s.publish(req).await.unwrap_err();
    assert!(
        matches!(err, CoreError::InvalidInput(_)),
        "traversal version must be rejected, got {err:?}"
    );
}

// ── yank / unyank role checks ─────────────────────────────────────────────

#[tokio::test]
async fn yank_requires_user_role() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .yank("cargo", "serde", "1.0.0", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

#[tokio::test]
async fn unyank_requires_user_role() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .unyank("cargo", "serde", "1.0.0", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

// ── npm packument / version not-found ─────────────────────────────────────

#[tokio::test]
async fn get_npm_packument_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_npm_packument("npm", "unknown", "http://localhost", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_npm_version_not_found_for_unknown_version() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("npm", "express", "4.0.0"));
    let s = svc(backend, None);
    let err = s
        .get_npm_version("npm", "express", "9.9.9", "http://localhost", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

// ── go module not-found ───────────────────────────────────────────────────

#[tokio::test]
async fn get_go_version_list_not_found_when_empty() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_go_version_list("go", "example.com/mod", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_go_info_not_found_for_unknown_version() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
    let s = svc(backend, None);
    let err = s
        .get_go_info("go", "example.com/mod", "v9.9.9", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_go_mod_not_found_for_unknown_version() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
    let s = svc(backend, None);
    let err = s
        .get_go_mod("go", "example.com/mod", "v9.9.9", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_go_mod_not_found_when_no_go_mod_key() {
    let backend = InMemBackend::arc();
    // Package exists but index_metadata has no "go_mod" key
    backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
    let s = svc(backend, None);
    let err = s
        .get_go_mod("go", "example.com/mod", "v1.0.0", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_go_latest_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_go_latest("go", "example.com/mod", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

// ── maven / nuget / pypi / composer not-found ────────────────────────────────

#[tokio::test]
async fn get_maven_versions_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_maven_versions("maven", "com.example:mylib", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_nuget_versions_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_nuget_versions("nuget", "Newtonsoft.Json", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_nuget_versions_returns_versions_when_published() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("nuget", "mylib", "1.0.0"));
    backend.seed(pkg("nuget", "mylib", "2.0.0"));
    let s = svc(backend, None);
    let versions = s
        .get_nuget_versions("nuget", "mylib", &anon())
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
}

#[tokio::test]
async fn get_pypi_simple_page_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_pypi_simple_page("pypi", "requests", "http://localhost", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn get_composer_p2_response_not_found_when_no_versions() {
    let s = svc(InMemBackend::arc(), None);
    let err = s
        .get_composer_p2_response("composer", "vendor/pkg", "http://localhost", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

// ── Beta channel ─────────────────────────────────────────────────────────────

/// Minimal in-memory BetaChannelPort whose membership set is seeded at construction.
struct MemBetaChannel {
    members: std::collections::HashSet<String>, // user_ids
}

impl MemBetaChannel {
    fn with_users(ids: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            members: ids.iter().map(|s| s.to_string()).collect(),
        })
    }
    fn empty() -> Arc<Self> {
        Arc::new(Self {
            members: std::collections::HashSet::new(),
        })
    }
}

#[async_trait]
impl crate::ports::BetaChannelPort for MemBetaChannel {
    async fn is_member(&self, _registry: &str, identity: &Identity) -> Result<bool, CoreError> {
        Ok(identity
            .user_id
            .as_ref()
            .map(|id| self.members.contains(id))
            .unwrap_or(false))
    }
    async fn add_member(
        &self,
        _: &str,
        _: crate::ports::BetaChannelEntry,
    ) -> Result<(), CoreError> {
        Ok(())
    }
    async fn remove_member(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn list_members(
        &self,
        _: &str,
    ) -> Result<Vec<crate::ports::BetaChannelEntry>, CoreError> {
        Ok(vec![])
    }
}

fn svc_with_beta(
    backend: Arc<InMemBackend>,
    beta: Arc<dyn crate::ports::BetaChannelPort>,
) -> LocalRegistryService {
    let mut bc = HashMap::new();
    bc.insert(
        "reg".to_owned(),
        beta as Arc<dyn crate::ports::BetaChannelPort>,
    );
    LocalRegistryService {
        backend,
        storage: Arc::new(NoopStorage),
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: bc,
            max_artifact_size_bytes: None,
        }),
        quota: None,
        ownership: None,
        team_namespace: None,
        sbom: None,
        explore_cache: None,
    }
}

fn beta_user() -> Identity {
    Identity {
        user_id: Some("beta".into()),
        role: Role::User,
        auth_provider: None,
        groups: vec![],
    }
}

// No beta channel configured → all versions visible to everyone (tested via npm packument).
#[tokio::test]
async fn filter_no_beta_channel_shows_all_versions() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "lib", "1.0.0"));
    backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
    let s = svc(backend, None);
    let doc = s
        .get_npm_packument("reg", "lib", "http://localhost", &anon())
        .await
        .unwrap();
    assert_eq!(doc["versions"].as_object().unwrap().len(), 2);
}

// Beta channel configured; anonymous user sees only stable versions.
#[tokio::test]
async fn filter_non_member_hides_prerelease() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "lib", "1.0.0"));
    backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
    let s = svc_with_beta(backend, MemBetaChannel::empty());
    let doc = s
        .get_npm_packument("reg", "lib", "http://localhost", &anon())
        .await
        .unwrap();
    let versions = doc["versions"].as_object().unwrap();
    assert_eq!(versions.len(), 1);
    assert!(versions.contains_key("1.0.0"));
}

// Beta channel configured; member sees all versions including pre-release.
#[tokio::test]
async fn filter_member_sees_prerelease() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "lib", "1.0.0"));
    backend.seed(pkg("reg", "lib", "1.1.0-beta.1"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let doc = s
        .get_npm_packument("reg", "lib", "http://localhost", &beta_user())
        .await
        .unwrap();
    assert_eq!(doc["versions"].as_object().unwrap().len(), 2);
}

// check_prerelease_access passes for stable versions regardless of membership.
#[tokio::test]
async fn check_prerelease_access_stable_always_ok() {
    let backend = InMemBackend::arc();
    let s = svc_with_beta(backend, MemBetaChannel::empty());
    s.check_prerelease_access("reg", "1.0.0", &anon())
        .await
        .unwrap();
}

// check_prerelease_access blocks non-members on pre-release versions.
#[tokio::test]
async fn check_prerelease_access_blocks_non_member() {
    let backend = InMemBackend::arc();
    let s = svc_with_beta(backend, MemBetaChannel::empty());
    let err = s
        .check_prerelease_access("reg", "1.1.0-beta.1", &anon())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

// check_prerelease_access allows members on pre-release versions.
#[tokio::test]
async fn check_prerelease_access_allows_member() {
    let backend = InMemBackend::arc();
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    s.check_prerelease_access("reg", "1.1.0-beta.1", &beta_user())
        .await
        .unwrap();
}

// check_prerelease_access passes when no beta channel is configured (open access).
#[tokio::test]
async fn check_prerelease_access_no_channel_open() {
    let backend = InMemBackend::arc();
    let s = svc(backend, None);
    s.check_prerelease_access("reg", "1.1.0-beta.1", &anon())
        .await
        .unwrap();
}

// npm packument: dist-tags.latest must point to latest stable, not pre-release.
#[tokio::test]
async fn npm_packument_latest_tag_skips_prerelease() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "pkg", "1.0.0"));
    backend.seed(pkg("reg", "pkg", "2.0.0-alpha.1"));
    // Even beta members should not see a pre-release as `latest`.
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let doc = s
        .get_npm_packument("reg", "pkg", "http://localhost", &beta_user())
        .await
        .unwrap();
    let latest = doc["dist-tags"]["latest"].as_str().unwrap();
    assert_eq!(latest, "1.0.0");
}

// npm packument: if all visible versions are pre-release, latest falls back to the newest pre-release.
#[tokio::test]
async fn npm_packument_latest_tag_only_prereleases() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "pkg", "1.0.0-beta.1"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let doc = s
        .get_npm_packument("reg", "pkg", "http://localhost", &beta_user())
        .await
        .unwrap();
    // No stable version; latest must fall back to the newest pre-release, not "".
    let latest = doc["dist-tags"]["latest"].as_str().unwrap();
    assert_eq!(latest, "1.0.0-beta.1");
}

// go @latest: prefers last stable; falls back to last pre-release only if no stable exists.
#[tokio::test]
async fn go_latest_prefers_stable_over_prerelease() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "mod", "1.0.0"));
    backend.seed(pkg("reg", "mod", "2.0.0-rc.1"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let info = s.get_go_latest("reg", "mod", &beta_user()).await.unwrap();
    assert_eq!(info["Version"].as_str().unwrap(), "1.0.0");
}

#[tokio::test]
async fn go_latest_falls_back_to_prerelease_when_all_prerelease() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "mod", "1.0.0-alpha.1"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let info = s.get_go_latest("reg", "mod", &beta_user()).await.unwrap();
    assert_eq!(info["Version"].as_str().unwrap(), "1.0.0-alpha.1");
}

// rubygems gem_info: same stable-preference behaviour.
#[tokio::test]
async fn rubygems_gem_info_prefers_stable() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "gem", "1.0.0"));
    backend.seed(pkg("reg", "gem", "1.1.0-pre"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let info = s
        .get_rubygems_gem_info("reg", "gem", &beta_user())
        .await
        .unwrap();
    assert_eq!(info["version"].as_str().unwrap(), "1.0.0");
}

// rubygems versions: prerelease field uses semver-aware detection.
#[tokio::test]
async fn rubygems_versions_prerelease_flag_uses_semver() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "gem", "1.0.0"));
    backend.seed(pkg("reg", "gem", "1.1.0-rc.1"));
    let s = svc_with_beta(backend, MemBetaChannel::with_users(&["beta"]));
    let versions = s
        .get_rubygems_versions("reg", "gem", &beta_user())
        .await
        .unwrap();
    // Newest first; 1.1.0-rc.1 is index 0.
    let pre = versions[0]["prerelease"].as_bool().unwrap();
    let stable = versions[1]["prerelease"].as_bool().unwrap();
    assert!(pre, "1.1.0-rc.1 should be marked prerelease=true");
    assert!(!stable, "1.0.0 should be marked prerelease=false");
}

// is_prerelease handles v-prefixed and Composer dev-branch versions.
#[test]
fn is_prerelease_handles_v_prefix_and_dev_branches() {
    let check = |v: &str| LocalRegistryService::is_prerelease(v);
    assert!(check("v1.0.0-beta.1"), "v-prefixed pre-release");
    assert!(check("dev-main"), "dev- prefix");
    assert!(check("dev-feature/branch"), "dev- with path");
    assert!(check("1.0.0-dev"), "-dev suffix");
    assert!(!check("v1.0.0"), "v-prefixed stable");
    assert!(!check("1.0.0"), "plain stable");
    assert!(!check("1.0.0.0"), "four-part (non-semver stable)");
}

// check_prerelease_access blocks non-members on Composer dev-branch versions.
#[tokio::test]
async fn check_prerelease_access_blocks_dev_branch_non_member() {
    let backend = InMemBackend::arc();
    let s = svc_with_beta(backend, MemBetaChannel::empty());
    let err = s
        .check_prerelease_access("reg", "dev-main", &anon())
        .await
        .unwrap_err();
    assert!(
        matches!(err, CoreError::NotFound(_)),
        "dev-main must be gated"
    );
}

// ── Team namespace enforcement tests ─────────────────────────────────────

#[derive(Debug, Default)]
struct MockTeamNamespace {
    namespaces: Mutex<Vec<crate::entities::TeamNamespace>>,
    visibility: Mutex<HashMap<(String, String), Visibility>>,
}

impl MockTeamNamespace {
    fn arc() -> Arc<Self> {
        Arc::new(Self::default())
    }
    fn with_namespace(registry: &str, prefix: &str, group: &str) -> Arc<Self> {
        let s = Self::arc();
        s.namespaces
            .lock()
            .unwrap()
            .push(crate::entities::TeamNamespace {
                registry: registry.to_owned(),
                prefix: prefix.to_owned(),
                group_id: group.to_owned(),
                claimed_by: None,
            });
        s
    }
    fn with_visibility(registry: &str, package: &str, vis: Visibility) -> Arc<Self> {
        let s = Self::arc();
        s.visibility
            .lock()
            .unwrap()
            .insert((registry.to_owned(), package.to_owned()), vis);
        s
    }
}

#[async_trait]
impl TeamNamespacePort for MockTeamNamespace {
    async fn find_namespace(
        &self,
        registry: &str,
        package: &str,
    ) -> Result<Option<crate::entities::TeamNamespace>, CoreError> {
        let ns = self.namespaces.lock().unwrap();
        let result = ns
            .iter()
            .filter(|n| {
                n.registry == registry
                    && (package == n.prefix
                        || (package.len() > n.prefix.len()
                            && package[..n.prefix.len() + 1] == format!("{}/", n.prefix)))
            })
            .max_by_key(|n| n.prefix.len())
            .cloned();
        Ok(result)
    }
    async fn list_namespaces(
        &self,
        _: &str,
    ) -> Result<Vec<crate::entities::TeamNamespace>, CoreError> {
        Ok(vec![])
    }
    async fn claim_namespace(&self, _: crate::entities::TeamNamespace) -> Result<(), CoreError> {
        Ok(())
    }
    async fn release_namespace(&self, _: &str, _: &str) -> Result<(), CoreError> {
        Ok(())
    }
    async fn set_visibility(&self, _: &str, _: &str, _: Visibility) -> Result<(), CoreError> {
        Ok(())
    }
    async fn get_visibility(&self, registry: &str, package: &str) -> Result<Visibility, CoreError> {
        Ok(self
            .visibility
            .lock()
            .unwrap()
            .get(&(registry.to_owned(), package.to_owned()))
            .cloned()
            .unwrap_or_default())
    }
    async fn list_namespaces_for_groups(
        &self,
        groups: &[String],
    ) -> Result<Vec<crate::entities::TeamNamespace>, CoreError> {
        let ns = self.namespaces.lock().unwrap();
        Ok(ns
            .iter()
            .filter(|n| {
                groups
                    .iter()
                    .any(|g| g.replace(' ', "") == n.group_id.replace(' ', ""))
            })
            .cloned()
            .collect())
    }
    async fn list_packages_in_namespace(
        &self,
        _: &str,
        _: &str,
        _: u64,
        _: u64,
    ) -> Result<Vec<crate::entities::NamespacePackage>, CoreError> {
        Ok(vec![])
    }
}

fn svc_with_ns(backend: Arc<InMemBackend>, ns: Arc<dyn TeamNamespacePort>) -> LocalRegistryService {
    LocalRegistryService {
        backend,
        storage: Arc::new(NoopStorage),
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        }),
        quota: None,
        ownership: None,
        team_namespace: Some(ns),
        sbom: None,
        explore_cache: None,
    }
}

fn member() -> Identity {
    Identity {
        user_id: Some("m1".into()),
        role: Role::User,
        auth_provider: None,
        groups: vec!["team-a".into()],
    }
}

fn non_member() -> Identity {
    Identity {
        user_id: Some("u2".into()),
        role: Role::User,
        auth_provider: None,
        groups: vec![],
    }
}

fn admin_id() -> Identity {
    Identity {
        user_id: Some("adm".into()),
        role: Role::Admin,
        auth_provider: None,
        groups: vec![],
    }
}

#[tokio::test]
async fn namespace_enforcement_blocks_non_member() {
    let backend = InMemBackend::arc();
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    let req = PublishRequest {
        registry: "reg".into(),
        name: "frontend/utils".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from("data"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: non_member(),
        signature_bytes: None,
        signature_type: None,
    };
    let err = s.publish(req).await.unwrap_err();
    assert!(
        matches!(err, CoreError::AccessDenied(_)),
        "non-member must be denied"
    );
}

#[tokio::test]
async fn namespace_enforcement_allows_member() {
    let backend = InMemBackend::arc();
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    let req = PublishRequest {
        registry: "reg".into(),
        name: "frontend/utils".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from("data"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: member(),
        signature_bytes: None,
        signature_type: None,
    };
    assert!(s.publish(req).await.is_ok(), "member must be allowed");
}

#[tokio::test]
async fn namespace_enforcement_admin_bypasses() {
    let backend = InMemBackend::arc();
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    let req = PublishRequest {
        registry: "reg".into(),
        name: "frontend/utils".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from("data"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: admin_id(),
        signature_bytes: None,
        signature_type: None,
    };
    assert!(
        s.publish(req).await.is_ok(),
        "admin must bypass namespace gate"
    );
}

#[tokio::test]
async fn no_namespace_claim_allows_any_user() {
    let backend = InMemBackend::arc();
    let ns = MockTeamNamespace::arc(); // no namespaces
    let s = svc_with_ns(backend, ns);
    let req = PublishRequest {
        registry: "reg".into(),
        name: "any/package".into(),
        version: "1.0.0".into(),
        artifact: Bytes::from("data"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: non_member(),
        signature_bytes: None,
        signature_type: None,
    };
    assert!(
        s.publish(req).await.is_ok(),
        "unclaimed namespace allows any user"
    );
}

// ── check_visibility tests ────────────────────────────────────────────────

#[tokio::test]
async fn visibility_public_allows_anonymous() {
    let s = svc(InMemBackend::arc(), None);
    // no team_namespace configured -> always Ok
    assert!(s.check_visibility("reg", "pkg", &anon()).await.is_ok());
}

#[tokio::test]
async fn visibility_internal_blocks_anonymous() {
    let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Internal);
    let s = svc_with_ns(InMemBackend::arc(), ns);
    let err = s.check_visibility("reg", "pkg", &anon()).await.unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

#[tokio::test]
async fn visibility_internal_allows_user() {
    let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Internal);
    let s = svc_with_ns(InMemBackend::arc(), ns);
    assert!(s
        .check_visibility("reg", "pkg", &non_member())
        .await
        .is_ok());
}

#[tokio::test]
async fn visibility_team_blocks_non_member() {
    let mock = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    // override visibility map
    let mock = {
        let inner = Arc::try_unwrap(mock).unwrap();
        inner
            .visibility
            .lock()
            .unwrap()
            .insert(("reg".into(), "frontend/pkg".into()), Visibility::Team);
        Arc::new(inner)
    };
    let s = svc_with_ns(InMemBackend::arc(), mock);
    let err = s
        .check_visibility("reg", "frontend/pkg", &non_member())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

#[tokio::test]
async fn visibility_team_allows_member() {
    let mock = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    mock.visibility
        .lock()
        .unwrap()
        .insert(("reg".into(), "frontend/pkg".into()), Visibility::Team);
    let s = svc_with_ns(InMemBackend::arc(), mock);
    assert!(s
        .check_visibility("reg", "frontend/pkg", &member())
        .await
        .is_ok());
}

#[tokio::test]
async fn visibility_admin_bypasses_all() {
    let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
    let s = svc_with_ns(InMemBackend::arc(), ns);
    assert!(s.check_visibility("reg", "pkg", &admin_id()).await.is_ok());
}

// When Team visibility is set but no namespace claim exists, access must
// be denied for ALL non-admins — falling back to "any authenticated user"
// would allow non-team members to read team-private packages.
#[tokio::test]
async fn visibility_team_no_claim_denies_authenticated_user() {
    // Visibility is Team but no namespace claim is seeded.
    let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
    let s = svc_with_ns(InMemBackend::arc(), ns);
    let err = s
        .check_visibility("reg", "pkg", &non_member())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

#[tokio::test]
async fn visibility_team_no_claim_denies_anonymous() {
    let ns = MockTeamNamespace::with_visibility("reg", "pkg", Visibility::Team);
    let s = svc_with_ns(InMemBackend::arc(), ns);
    let err = s.check_visibility("reg", "pkg", &anon()).await.unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

// Verify visibility is inherited when a second version is published on a
// package that already has a non-public visibility.
#[tokio::test]
async fn publish_second_version_inherits_visibility() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "my-pkg", "1.0.0"));

    // Seed visibility = Internal for the first version.
    let ns = MockTeamNamespace::arc();
    ns.visibility
        .lock()
        .unwrap()
        .insert(("reg".into(), "my-pkg".into()), Visibility::Internal);
    let s = svc_with_ns(backend, ns);

    let req = PublishRequest {
        registry: "reg".into(),
        name: "my-pkg".into(),
        version: "2.0.0".into(),
        artifact: bytes::Bytes::from("data"),
        checksum: "abc".into(),
        index_metadata: serde_json::json!({}),
        publisher: user(),
        signature_bytes: None,
        signature_type: None,
    };
    s.publish(req).await.unwrap();

    // The newly published version must carry the inherited visibility.
    let versions = s.backend.get_versions("reg", "my-pkg").await.unwrap();
    let v2 = versions.iter().find(|v| v.version == "2.0.0").unwrap();
    assert_eq!(
        v2.visibility,
        Visibility::Internal,
        "second version must inherit Internal visibility from the package"
    );
}

// ── yank/unyank namespace enforcement ────────────────────────────────────

#[tokio::test]
async fn yank_blocks_non_member_in_claimed_namespace() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    let err = s
        .yank("reg", "frontend/utils", "1.0.0", &non_member())
        .await
        .unwrap_err();
    assert!(
        matches!(err, CoreError::AccessDenied(_)),
        "non-member must not yank namespace package"
    );
}

#[tokio::test]
async fn yank_allows_namespace_member() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    assert!(s
        .yank("reg", "frontend/utils", "1.0.0", &member())
        .await
        .is_ok());
}

#[tokio::test]
async fn yank_admin_bypasses_namespace() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    assert!(s
        .yank("reg", "frontend/utils", "1.0.0", &admin_id())
        .await
        .is_ok());
}

#[tokio::test]
async fn yank_unclaimed_package_allows_any_user() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "unclaimed/pkg", "1.0.0"));
    let ns = MockTeamNamespace::arc(); // no claims
    let s = svc_with_ns(backend, ns);
    assert!(s
        .yank("reg", "unclaimed/pkg", "1.0.0", &non_member())
        .await
        .is_ok());
}

#[tokio::test]
async fn unyank_blocks_non_member_in_claimed_namespace() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    let err = s
        .unyank("reg", "frontend/utils", "1.0.0", &non_member())
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::AccessDenied(_)));
}

#[tokio::test]
async fn unyank_allows_namespace_member() {
    let backend = InMemBackend::arc();
    backend.seed(pkg("reg", "frontend/utils", "1.0.0"));
    let ns = MockTeamNamespace::with_namespace("reg", "frontend", "team-a");
    let s = svc_with_ns(backend, ns);
    assert!(s
        .unyank("reg", "frontend/utils", "1.0.0", &member())
        .await
        .is_ok());
}
