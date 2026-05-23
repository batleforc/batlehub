use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;

use crate::{
    entities::{Identity, PublishedPackage, Role},
    error::CoreError,
    ports::{LocalRegistryBackend, StorageBackend, StorageMeta},
    services::quota::{QuotaCheck, QuotaService},
};

/// Input to `LocalRegistryService::publish`.
pub struct PublishRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// Raw artifact bytes.
    pub artifact: Bytes,
    /// SHA-256 hex of `artifact`, computed by the caller (handler layer).
    pub checksum: String,
    /// Ecosystem-specific index metadata serialised as JSON.
    /// Cargo: serialised `CargoIndexEntry` (with `cksum` already set).
    /// npm: version metadata from the publish payload (`dist.tarball` stripped).
    /// VSIX: `{"id": "pub.name", "version": "1.0.0"}`.
    pub index_metadata: serde_json::Value,
    /// Identity of the publishing user.
    pub publisher: Identity,
}

/// Authoritative local-registry service: publish, yank, index, artifact retrieval.
pub struct LocalRegistryService {
    pub backend: Arc<dyn LocalRegistryBackend>,
    pub storage: Arc<dyn StorageBackend>,
    /// Maximum artifact size in bytes. Defaults to 500 MiB when `None`.
    pub max_artifact_bytes: Option<u64>,
    /// Optional publish quota enforcement. When `None`, quotas are disabled.
    pub quota: Option<Arc<QuotaService>>,
}

impl LocalRegistryService {
    /// Validate and persist a published artifact.
    ///
    /// Returns a `QuotaCheck` describing the publisher's current quota state
    /// after the publish (useful for setting `X-Quota-*` response headers).
    /// Returns a zeroed `QuotaCheck` when no quota is configured.
    pub async fn publish(&self, req: PublishRequest) -> Result<QuotaCheck, CoreError> {
        if !req.publisher.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "publishing requires at least User role".into(),
            ));
        }

        let limit = self.max_artifact_bytes.unwrap_or(500 * 1024 * 1024);
        if req.artifact.len() as u64 > limit {
            return Err(CoreError::PayloadTooLarge(format!(
                "artifact is {} bytes; limit is {}",
                req.artifact.len(),
                limit
            )));
        }

        // Check and record quota before persisting. This may return QuotaExceeded.
        let quota_check = if let Some(quota_svc) = &self.quota {
            quota_svc
                .check_and_record_publish(&req.publisher, &req.registry, req.artifact.len() as u64)
                .await?
        } else {
            QuotaCheck::default()
        };

        let pkg = PublishedPackage {
            registry: req.registry.clone(),
            name: req.name.clone(),
            version: req.version.clone(),
            checksum: req.checksum.clone(),
            yanked: false,
            index_metadata: req.index_metadata,
            published_at: chrono::Utc::now(),
            published_by: req.publisher.user_id.clone(),
        };

        let storage_key = artifact_storage_key(&req.registry, &req.name, &req.version);
        let bytes = req.artifact.len() as u64;

        // Step 1: reserve the version (inserted as 'pending', invisible to readers).
        if let Err(e) = self.backend.publish(pkg).await {
            // Row was not inserted; only quota needs rollback.
            self.revoke_quota(&req.publisher, &req.registry, bytes).await;
            return Err(e);
        }

        // Step 2: persist artifact bytes. On failure, discard the pending row.
        if let Err(e) = self
            .storage
            .store(
                &storage_key,
                req.artifact.clone(),
                StorageMeta {
                    content_type: Some("application/octet-stream".into()),
                    size: None,
                    checksum: Some(req.checksum.clone()),
                },
            )
            .await
        {
            self.remove_pending(&req.registry, &req.name, &req.version).await;
            self.revoke_quota(&req.publisher, &req.registry, bytes).await;
            return Err(e);
        }

        // Step 3: promote the pending row to 'published'. On failure, undo both
        // the storage write and the pending row so the caller gets a clean error.
        if let Err(e) = self
            .backend
            .commit_publish(&req.registry, &req.name, &req.version)
            .await
        {
            self.remove_pending(&req.registry, &req.name, &req.version).await;
            if let Err(err) = self.storage.delete(&storage_key).await {
                tracing::error!("storage cleanup after commit failure: {err}");
            }
            self.revoke_quota(&req.publisher, &req.registry, bytes).await;
            return Err(e);
        }

        Ok(quota_check)
    }

    pub async fn yank(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "yank requires at least User role".into(),
            ));
        }
        self.backend.yank(registry, name, version).await
    }

    pub async fn unyank(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        identity: &Identity,
    ) -> Result<(), CoreError> {
        if !identity.has_role_at_least(&Role::User) {
            return Err(CoreError::AccessDenied(
                "unyank requires at least User role".into(),
            ));
        }
        self.backend.unyank(registry, name, version).await
    }

    async fn remove_pending(&self, registry: &str, name: &str, version: &str) {
        if let Err(err) = self.backend.remove_version(registry, name, version).await {
            tracing::error!("pending row cleanup failed: {err}");
        }
    }

    async fn revoke_quota(&self, identity: &Identity, registry: &str, bytes: u64) {
        if let Some(svc) = &self.quota {
            if let Err(err) = svc.revoke_publish(identity, registry, bytes).await {
                tracing::error!("quota revoke failed: {err}");
            }
        }
    }

    /// Return the sparse index file content (newline-delimited JSON) for a Cargo crate.
    /// Returns `CoreError::NotFound` if the crate has never been published here.
    pub async fn get_index(&self, registry: &str, name: &str) -> Result<String, CoreError> {
        let versions = self.backend.get_versions(registry, name).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "crate '{}' not found in local registry '{}'",
                name, registry
            )));
        }
        let lines = versions
            .iter()
            .map(|v| serde_json::to_string(&v.index_metadata))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| CoreError::Registry(e.to_string()))?;
        Ok(lines.join("\n"))
    }

    /// Build an npm packument for all published versions, rewriting `dist.tarball`
    /// to point at `base_url` (e.g. `"https://batlehub.example.com"`).
    pub async fn get_npm_packument(
        &self,
        registry: &str,
        name: &str,
        base_url: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.backend.get_versions(registry, name).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "package '{}' not found in local registry '{}'",
                name, registry
            )));
        }

        let base = base_url.trim_end_matches('/');
        let mut versions_map = serde_json::Map::new();
        let mut time_map = serde_json::Map::new();
        let mut latest = String::new();

        for pkg in &versions {
            let mut meta = pkg.index_metadata.clone();
            if let Some(obj) = meta.as_object_mut() {
                let dist = obj
                    .entry("dist")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(d) = dist.as_object_mut() {
                    d.insert(
                        "tarball".to_owned(),
                        serde_json::json!(format!(
                            "{base}/proxy/{registry}/{name}/{version}/tarball",
                            version = pkg.version
                        )),
                    );
                }
            }
            time_map.insert(
                pkg.version.clone(),
                serde_json::json!(pkg.published_at.to_rfc3339()),
            );
            versions_map.insert(pkg.version.clone(), meta);
            latest = pkg.version.clone();
        }

        Ok(serde_json::json!({
            "name": name,
            "_id": name,
            "dist-tags": { "latest": latest },
            "versions": versions_map,
            "time": time_map
        }))
    }

    /// Return a single npm version metadata object with `dist.tarball` rewritten.
    pub async fn get_npm_version(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        base_url: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.backend.get_versions(registry, name).await?;
        let pkg = versions
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    name, version, registry
                ))
            })?;

        let base = base_url.trim_end_matches('/');
        let mut meta = pkg.index_metadata.clone();
        if let Some(obj) = meta.as_object_mut() {
            let dist = obj
                .entry("dist")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(d) = dist.as_object_mut() {
                d.insert(
                    "tarball".to_owned(),
                    serde_json::json!(format!(
                        "{base}/proxy/{registry}/{name}/{version}/tarball"
                    )),
                );
            }
        }
        Ok(meta)
    }

    /// Retrieve the raw artifact bytes for download.
    pub async fn get_artifact(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<Bytes, CoreError> {
        let key = artifact_storage_key(registry, name, version);
        let artifact = self
            .storage
            .retrieve(&key)
            .await?
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}/{}@{} not found in local registry",
                    registry, name, version
                ))
            })?;
        let mut buf = Vec::new();
        let mut stream = artifact.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk?);
        }
        Ok(Bytes::from(buf))
    }

    /// Return newline-delimited version list for a locally published Go module.
    /// Returns `CoreError::NotFound` if the module has never been published here.
    pub async fn get_go_version_list(&self, registry: &str, module: &str) -> Result<String, CoreError> {
        let versions = self.backend.get_versions(registry, module).await?;
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "module '{}' not found in local registry '{}'",
                module, registry
            )));
        }
        let list = versions
            .iter()
            .filter_map(|v| {
                v.index_metadata
                    .get("Version")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(list)
    }

    /// Return the `.info` JSON for a specific Go module version.
    pub async fn get_go_info(
        &self,
        registry: &str,
        module: &str,
        version: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let pkg = self
            .backend
            .get_versions(registry, module)
            .await?
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    module, version, registry
                ))
            })?;
        let v = pkg
            .index_metadata
            .get("Version")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(version));
        let t = pkg
            .index_metadata
            .get("Time")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.published_at.to_rfc3339()));
        Ok(serde_json::json!({ "Version": v, "Time": t }))
    }

    /// Return the `go.mod` content for a specific Go module version.
    pub async fn get_go_mod(
        &self,
        registry: &str,
        module: &str,
        version: &str,
    ) -> Result<String, CoreError> {
        let pkg = self
            .backend
            .get_versions(registry, module)
            .await?
            .into_iter()
            .find(|v| v.version == version)
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "{}@{} not found in local registry '{}'",
                    module, version, registry
                ))
            })?;
        pkg.index_metadata
            .get("go_mod")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .ok_or_else(|| {
                CoreError::NotFound(format!(
                    "go.mod not found for {}@{} in registry '{}'",
                    module, version, registry
                ))
            })
    }

    /// Return the `.info` JSON for the most recently published Go module version.
    pub async fn get_go_latest(
        &self,
        registry: &str,
        module: &str,
    ) -> Result<serde_json::Value, CoreError> {
        let versions = self.backend.get_versions(registry, module).await?;
        let pkg = versions.into_iter().last().ok_or_else(|| {
            CoreError::NotFound(format!(
                "module '{}' not found in local registry '{}'",
                module, registry
            ))
        })?;
        let v = pkg
            .index_metadata
            .get("Version")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.version));
        let t = pkg
            .index_metadata
            .get("Time")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(pkg.published_at.to_rfc3339()));
        Ok(serde_json::json!({ "Version": v, "Time": t }))
    }
}

/// Stable storage key for a locally published artifact.
/// Distinct from the proxy `artifact:…` namespace to avoid collisions.
pub fn artifact_storage_key(registry: &str, name: &str, version: &str) -> String {
    format!("local:{}/{}/{}", registry, name, version)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use chrono::Utc;

    use super::*;
    use crate::{
        entities::{Identity, Role},
        error::CoreError,
        ports::{StorageBackend, StorageMeta, StoredArtifact},
    };

    // ── Minimal mock backend ──────────────────────────────────────────────────

    #[derive(Default)]
    struct InMemBackend {
        versions: Mutex<Vec<PublishedPackage>>,
    }

    impl InMemBackend {
        fn arc() -> Arc<Self> { Arc::new(Self::default()) }
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
        async fn yank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> { Ok(()) }
        async fn unyank(&self, _: &str, _: &str, _: &str) -> Result<(), CoreError> { Ok(()) }
        async fn get_versions(&self, registry: &str, name: &str) -> Result<Vec<PublishedPackage>, CoreError> {
            Ok(self.versions.lock().unwrap().iter()
                .filter(|p| p.registry == registry && p.name == name)
                .cloned()
                .collect())
        }
        async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
            Ok(self.versions.lock().unwrap().iter().any(|p| p.registry == registry && p.name == name))
        }
    }

    struct NoopStorage;

    #[async_trait]
    impl StorageBackend for NoopStorage {
        async fn store(&self, _: &str, _: Bytes, _: StorageMeta) -> Result<(), CoreError> { Ok(()) }
        async fn retrieve(&self, _: &str) -> Result<Option<StoredArtifact>, CoreError> { Ok(None) }
        async fn exists(&self, _: &str) -> Result<bool, CoreError> { Ok(false) }
        async fn delete(&self, _: &str) -> Result<(), CoreError> { Ok(()) }
        async fn delete_by_prefix(&self, _: &str) -> Result<usize, CoreError> { Ok(0) }
        async fn stat_by_prefix(&self, _: &str) -> Result<(u64, u64), CoreError> { Ok((0, 0)) }
        async fn list_keys(&self, _: &str) -> Result<Vec<String>, CoreError> { Ok(vec![]) }
    }

    fn svc(backend: Arc<InMemBackend>, max_bytes: Option<u64>) -> LocalRegistryService {
        LocalRegistryService {
            backend,
            storage: Arc::new(NoopStorage),
            max_artifact_bytes: max_bytes,
            quota: None,
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
        }
    }

    fn anon() -> Identity {
        Identity { user_id: None, role: Role::Anonymous, auth_provider: None, groups: vec![] }
    }

    fn user() -> Identity {
        Identity { user_id: Some("u1".into()), role: Role::User, auth_provider: None, groups: vec![] }
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
        };
        let err = s.publish(req).await.unwrap_err();
        assert!(matches!(err, CoreError::PayloadTooLarge(_)));
    }

    // ── yank / unyank role checks ─────────────────────────────────────────────

    #[tokio::test]
    async fn yank_requires_user_role() {
        let s = svc(InMemBackend::arc(), None);
        let err = s.yank("cargo", "serde", "1.0.0", &anon()).await.unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    #[tokio::test]
    async fn unyank_requires_user_role() {
        let s = svc(InMemBackend::arc(), None);
        let err = s.unyank("cargo", "serde", "1.0.0", &anon()).await.unwrap_err();
        assert!(matches!(err, CoreError::AccessDenied(_)));
    }

    // ── npm packument / version not-found ─────────────────────────────────────

    #[tokio::test]
    async fn get_npm_packument_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s.get_npm_packument("npm", "unknown", "http://localhost").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_npm_version_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("npm", "express", "4.0.0"));
        let s = svc(backend, None);
        let err = s.get_npm_version("npm", "express", "9.9.9", "http://localhost").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    // ── go module not-found ───────────────────────────────────────────────────

    #[tokio::test]
    async fn get_go_version_list_not_found_when_empty() {
        let s = svc(InMemBackend::arc(), None);
        let err = s.get_go_version_list("go", "example.com/mod").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_info_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s.get_go_info("go", "example.com/mod", "v9.9.9").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_mod_not_found_for_unknown_version() {
        let backend = InMemBackend::arc();
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s.get_go_mod("go", "example.com/mod", "v9.9.9").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_mod_not_found_when_no_go_mod_key() {
        let backend = InMemBackend::arc();
        // Package exists but index_metadata has no "go_mod" key
        backend.seed(pkg("go", "example.com/mod", "v1.0.0"));
        let s = svc(backend, None);
        let err = s.get_go_mod("go", "example.com/mod", "v1.0.0").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_go_latest_not_found_when_no_versions() {
        let s = svc(InMemBackend::arc(), None);
        let err = s.get_go_latest("go", "example.com/mod").await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }
}
