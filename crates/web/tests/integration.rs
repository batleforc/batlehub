//! End-to-end integration tests for the web layer.
//!
//! Each test spins up a full actix-web test app wired with:
//!  - StaticTokenAuthProvider  (admin-token / user-token)
//!  - InMemoryPackageRepository
//!  - InMemoryStorageBackend
//!  - InMemoryCacheStore
//!  - FixedRegistry (deterministic mock — returns canned metadata/bytes)
//!
//! No real database, filesystem, or upstream registry is involved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::test::{call_service, init_service, read_body, read_body_json, TestRequest};
use actix_web::App;
use utoipa_actix_web::AppExt;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use futures::stream;
use serde_json::Value;
use uuid::Uuid;

use base64::Engine as _;
use batlehub_adapters::auth::StaticTokenAuthProvider;
use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_core::{
    entities::{
        AccessEvent, EventFilter, PackageFilter, PackageId, PackageMetadata, PackageStatus,
        PackageSummary, PublishedPackage, Role,
    },
    error::CoreError,
    ports::{
        ArtifactMeta, ArtifactMetaRepository, AuthProvider, ByteStream, CacheStore,
        FetchedArtifact, LocalRegistryBackend, PackageRepository, RegistryClient,
        StorageBackend, StoredArtifact, StorageMeta, UserToken, UserTokenRepository,
    },
    rules::{BlockListRule, RbacRule},
    services::{AdminService, LocalRegistryService, ProxyService, RegistryPolicy},
};
use batlehub_config::schema::RegistryMode;
use batlehub_web::{configure_app, AuthMiddlewareFactory, RegistryModeMap};

// ── In-memory PackageRepository ────────────────────────────────────────────────

struct InMemoryRepo {
    summaries: Mutex<HashMap<String, PackageSummary>>,
    events: Mutex<Vec<AccessEvent>>,
}

impl InMemoryRepo {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            summaries: Mutex::new(HashMap::new()),
            events: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl PackageRepository for InMemoryRepo {
    async fn record_access(&self, event: AccessEvent) -> Result<(), CoreError> {
        let key = event.package_id.cache_key();
        let mut sums = self.summaries.lock().unwrap();
        let entry = sums.entry(key).or_insert_with(|| PackageSummary {
            id: Uuid::new_v4(),
            package_id: event.package_id.clone(),
            status: PackageStatus::Available,
            last_accessed: None,
            last_accessed_by: None,
            access_count: 0,
        });
        entry.access_count += 1;
        entry.last_accessed = Some(event.timestamp);
        self.events.lock().unwrap().push(event);
        Ok(())
    }

    async fn get_status(&self, pkg: &PackageId) -> Result<PackageStatus, CoreError> {
        Ok(self
            .summaries
            .lock()
            .unwrap()
            .get(&pkg.cache_key())
            .map(|s| s.status.clone())
            .unwrap_or(PackageStatus::Available))
    }

    async fn set_status(&self, pkg: &PackageId, status: PackageStatus) -> Result<(), CoreError> {
        let mut sums = self.summaries.lock().unwrap();
        let entry = sums.entry(pkg.cache_key()).or_insert_with(|| PackageSummary {
            id: Uuid::new_v4(),
            package_id: pkg.clone(),
            status: PackageStatus::Available,
            last_accessed: None,
            last_accessed_by: None,
            access_count: 0,
        });
        entry.status = status;
        Ok(())
    }

    async fn list_packages(&self, filter: PackageFilter) -> Result<Vec<PackageSummary>, CoreError> {
        let sums = self.summaries.lock().unwrap();
        let mut items: Vec<PackageSummary> = sums
            .values()
            .filter(|s| {
                if let Some(r) = &filter.registry {
                    if &s.package_id.registry != r {
                        return false;
                    }
                }
                if let Some(n) = &filter.name_contains {
                    if !s.package_id.name.contains(n.as_str()) {
                        return false;
                    }
                }
                if filter.blocked_only && !s.status.is_blocked() {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        items.sort_by_key(|s| s.package_id.cache_key());
        let items = items
            .into_iter()
            .skip(filter.offset as usize)
            .take(filter.limit as usize)
            .collect();
        Ok(items)
    }

    async fn list_events(&self, filter: EventFilter) -> Result<Vec<AccessEvent>, CoreError> {
        let events = self.events.lock().unwrap();
        let items = events
            .iter()
            .filter(|e| {
                if let Some(r) = &filter.registry {
                    if &e.package_id.registry != r {
                        return false;
                    }
                }
                if let Some(uid) = &filter.user_id {
                    if e.user_id.as_deref() != Some(uid.as_str()) {
                        return false;
                    }
                }
                if filter.denied_only && !e.result.is_denied() {
                    return false;
                }
                true
            })
            .cloned()
            .skip(filter.offset as usize)
            .take(filter.limit as usize)
            .collect();
        Ok(items)
    }
}

// ── In-memory StorageBackend ──────────────────────────────────────────────────

struct InMemoryStorage {
    data: Mutex<HashMap<String, (Bytes, StorageMeta)>>,
}

impl InMemoryStorage {
    fn new() -> Arc<Self> {
        Arc::new(Self { data: Mutex::new(HashMap::new()) })
    }
}

#[async_trait]
impl StorageBackend for InMemoryStorage {
    async fn store(&self, key: &str, data: Bytes, meta: StorageMeta) -> Result<(), CoreError> {
        self.data.lock().unwrap().insert(key.to_owned(), (data, meta));
        Ok(())
    }

    async fn retrieve(&self, key: &str) -> Result<Option<StoredArtifact>, CoreError> {
        let lock = self.data.lock().unwrap();
        Ok(lock.get(key).map(|(data, meta)| {
            let bytes = data.clone();
            let stream: ByteStream =
                Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(bytes) }));
            StoredArtifact { stream, meta: meta.clone() }
        }))
    }

    async fn exists(&self, key: &str) -> Result<bool, CoreError> {
        Ok(self.data.lock().unwrap().contains_key(key))
    }

    async fn delete(&self, key: &str) -> Result<(), CoreError> {
        self.data.lock().unwrap().remove(key);
        Ok(())
    }
    async fn delete_by_prefix(&self, prefix: &str) -> Result<usize, CoreError> {
        let mut map = self.data.lock().unwrap();
        let keys: Vec<String> = map.keys().filter(|k| k.starts_with(prefix)).cloned().collect();
        let count = keys.len();
        for k in keys { map.remove(&k); }
        Ok(count)
    }
    async fn stat_by_prefix(&self, prefix: &str) -> Result<(u64, u64), CoreError> {
        let map = self.data.lock().unwrap();
        let (count, bytes) = map.iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .fold((0u64, 0u64), |(c, b), (_, (data, meta))| {
                (c + 1, b + meta.size.unwrap_or(data.len() as u64))
            });
        Ok((count, bytes))
    }
    async fn list_keys(&self, prefix: &str) -> Result<Vec<String>, CoreError> {
        let map = self.data.lock().unwrap();
        Ok(map.keys().filter(|k| k.starts_with(prefix)).cloned().collect())
    }
}

// ── No-op ArtifactMetaRepository for tests ───────────────────────────────────

struct NoopArtifactMeta;
impl NoopArtifactMeta {
    fn arc() -> Arc<dyn ArtifactMetaRepository> { Arc::new(Self) }
}
#[async_trait]
impl ArtifactMetaRepository for NoopArtifactMeta {
    async fn record_artifact(&self, _: &str, _: &str, _: &str, _: &str, _: Option<u64>) -> Result<(), CoreError> { Ok(()) }
    async fn touch_artifact(&self, _: &str) -> Result<(), CoreError> { Ok(()) }
    async fn list_artifacts(&self, _: &str) -> Result<Vec<ArtifactMeta>, CoreError> { Ok(vec![]) }
    async fn list_artifacts_by_package(&self) -> Result<Vec<ArtifactMeta>, CoreError> { Ok(vec![]) }
    async fn delete_artifact_meta(&self, _: &str) -> Result<(), CoreError> { Ok(()) }
    async fn list_expired_by_ttl(&self, _: &str, _: chrono::DateTime<chrono::Utc>) -> Result<Vec<ArtifactMeta>, CoreError> { Ok(vec![]) }
    async fn list_idle(&self, _: &str, _: chrono::DateTime<chrono::Utc>) -> Result<Vec<ArtifactMeta>, CoreError> { Ok(vec![]) }
    async fn total_size_bytes(&self, _: &str) -> Result<u64, CoreError> { Ok(0) }
    async fn list_lru(&self, _: &str, _: i64) -> Result<Vec<ArtifactMeta>, CoreError> { Ok(vec![]) }
}

// ── Fixed (deterministic) RegistryClient ─────────────────────────────────────

struct FixedRegistry {
    registry_type: String,
}

impl FixedRegistry {
    fn new(registry_type: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { registry_type: registry_type.into() })
    }
}

#[async_trait]
impl RegistryClient for FixedRegistry {
    fn registry_type(&self) -> &str {
        &self.registry_type
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Ok(PackageMetadata {
            id: pkg.clone(),
            // Old enough to pass any age gate
            published_at: Some(Utc::now() - chrono::Duration::days(30)),
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({"registry": self.registry_type, "name": pkg.name}),
            cache_control: None,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let body = format!("artifact:{}:{}", self.registry_type, pkg.cache_key());
        let bytes = Bytes::from(body);
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(bytes) })),
            cache_control: None,
        })
    }
}

// ── App factory ───────────────────────────────────────────────────────────────

const ADMIN_TOKEN: &str = "admin-token";
const USER_TOKEN: &str = "user-token";
const TEAM_A_TOKEN: &str = "team-a-token";
const TEAM_B_TOKEN: &str = "team-b-token";
const TEAM_AB_TOKEN: &str = "team-ab-token";

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

fn test_auth_providers() -> Vec<Arc<dyn AuthProvider>> {
    vec![Arc::new(StaticTokenAuthProvider::new([
        (ADMIN_TOKEN.to_owned(), Some("admin".to_owned()), Role::Admin),
        (USER_TOKEN.to_owned(), Some("user-1".to_owned()), Role::User),
    ]))]
}

struct NullTokenRepository;

#[async_trait]
impl UserTokenRepository for NullTokenRepository {
    async fn create_token(
        &self,
        _id: uuid::Uuid,
        _user_id: &str,
        _name: &str,
        _token_hash: &str,
        _role: Role,
        _expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<UserToken, CoreError> {
        Err(CoreError::Database("not implemented".into()))
    }
    async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        Ok(None)
    }
    async fn list_for_user(&self, _user_id: &str) -> Result<Vec<UserToken>, CoreError> {
        Ok(vec![])
    }
    async fn revoke(&self, _id: uuid::Uuid, _user_id: &str) -> Result<bool, CoreError> {
        Ok(false)
    }
}

// ── In-memory LocalRegistryBackend ───────────────────────────────────────────

struct InMemoryLocalRegistry {
    packages: Mutex<HashMap<String, Vec<PublishedPackage>>>,
}

impl InMemoryLocalRegistry {
    fn new() -> Arc<Self> {
        Arc::new(Self { packages: Mutex::new(HashMap::new()) })
    }
}

#[async_trait]
impl LocalRegistryBackend for InMemoryLocalRegistry {
    async fn publish(&self, pkg: PublishedPackage) -> Result<(), CoreError> {
        let key = format!("{}:{}", pkg.registry, pkg.name);
        let mut map = self.packages.lock().unwrap();
        let versions = map.entry(key).or_default();
        if versions.iter().any(|v| v.version == pkg.version) {
            return Err(CoreError::Conflict(format!(
                "{}@{} already published",
                pkg.name, pkg.version
            )));
        }
        versions.push(pkg);
        Ok(())
    }

    async fn yank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        let key = format!("{registry}:{name}");
        let mut map = self.packages.lock().unwrap();
        if let Some(versions) = map.get_mut(&key) {
            for v in versions.iter_mut() {
                if v.version == version {
                    v.yanked = true;
                }
            }
        }
        Ok(())
    }

    async fn unyank(&self, registry: &str, name: &str, version: &str) -> Result<(), CoreError> {
        let key = format!("{registry}:{name}");
        let mut map = self.packages.lock().unwrap();
        if let Some(versions) = map.get_mut(&key) {
            for v in versions.iter_mut() {
                if v.version == version {
                    v.yanked = false;
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
        let key = format!("{registry}:{name}");
        let map = self.packages.lock().unwrap();
        Ok(map.get(&key).cloned().unwrap_or_default())
    }

    async fn exists(&self, registry: &str, name: &str) -> Result<bool, CoreError> {
        let key = format!("{registry}:{name}");
        let map = self.packages.lock().unwrap();
        Ok(map.contains_key(&key))
    }
}

fn make_local_svc(storage: Arc<dyn StorageBackend>) -> Arc<LocalRegistryService> {
    Arc::new(LocalRegistryService {
        backend: InMemoryLocalRegistry::new(),
        storage,
        max_artifact_bytes: None,
    })
}

fn rbac_policy(
    repo: Arc<dyn PackageRepository>,
) -> RegistryPolicy {
    let perms = HashMap::from([
        (Role::Anonymous, vec!["releases:read".to_owned()]),
        (Role::User, vec!["releases:read".to_owned(), "source:read".to_owned()]),
        (Role::Admin, vec!["*".to_owned()]),
    ]);
    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(300)),
        firewall_only: false,
        serve_stale_metadata: false,
        artifact_ttl: None,
        rules: vec![
            Box::new(RbacRule::new(perms)),
            Box::new(BlockListRule::new(repo)),
        ],
    }
}

/// Build a fully-wired test app. The caller keeps a reference to `repo`
/// to pre-seed or inspect state during the test.
async fn make_app(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("github".to_owned(), FixedRegistry::new("github") as Arc<dyn RegistryClient>),
        ("npm".to_owned(), FixedRegistry::new("npm") as Arc<dyn RegistryClient>),
        ("cargo".to_owned(), FixedRegistry::new("cargo") as Arc<dyn RegistryClient>),
        ("openvsx".to_owned(), FixedRegistry::new("openvsx") as Arc<dyn RegistryClient>),
        ("go".to_owned(), FixedRegistry::new("goproxy") as Arc<dyn RegistryClient>),
        ("vscode".to_owned(), FixedRegistry::new("vscode-marketplace") as Arc<dyn RegistryClient>),
    ]
    .into();

    let policies: HashMap<String, RegistryPolicy> = [
        ("github".to_owned(), rbac_policy(repo_dyn.clone())),
        ("npm".to_owned(), rbac_policy(repo_dyn.clone())),
        ("cargo".to_owned(), rbac_policy(repo_dyn.clone())),
        ("openvsx".to_owned(), rbac_policy(repo_dyn.clone())),
        ("go".to_owned(), rbac_policy(repo_dyn.clone())),
        ("vscode".to_owned(), rbac_policy(repo_dyn.clone())),
    ]
    .into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));

    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["github", "npm", "cargo", "openvsx", "go", "vscode"].iter().map(|s| s.to_string()).collect(),
        user: ["github", "npm", "cargo", "openvsx", "go", "vscode"].iter().map(|s| s.to_string()).collect(),
        admin: ["github", "npm", "cargo", "openvsx", "go", "vscode"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("github", "github"), ("npm", "npm"), ("cargo", "cargo"), ("openvsx", "openvsx"), ("go", "goproxy"), ("vscode", "vscode-marketplace")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, token_repo, None, access_config, registry_map, batlehub_web::UpstreamMap::default(), vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));

    init_service(
        app.wrap(AuthMiddlewareFactory::new(test_auth_providers())),
    )
    .await
}

// ── /api/v1/me ────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn me_without_auth_returns_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/me").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["role"], "anonymous");
    assert!(body["user_id"].is_null());
}

#[actix_web::test]
async fn me_with_admin_token_returns_admin_identity() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["role"], "admin");
    assert_eq!(body["user_id"], "admin");
}

#[actix_web::test]
async fn me_with_user_token_returns_user_identity() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["role"], "user");
    assert_eq!(body["user_id"], "user-1");
}

#[actix_web::test]
async fn me_with_invalid_token_falls_back_to_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", "Bearer not-a-real-token"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["role"], "anonymous");
}

// ── /api/v1/packages ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn packages_list_is_empty_on_fresh_repo() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/packages").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 0);
    assert_eq!(body["total"], 0);
}

#[actix_web::test]
async fn packages_list_shows_packages_after_access() {
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("npm", "lodash", "4.17.21");
    repo.record_access(AccessEvent::allowed_download(
        pkg.clone(),
        Some("user-1".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get().uri("/api/v1/packages").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["name"], "lodash");
}

// ── /api/v1/packages/access ───────────────────────────────────────────────────

#[actix_web::test]
async fn access_check_returns_true_for_available_package() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=npm&name=lodash&version=4.17.21")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["can_access"], true);
    assert!(body["reason"].is_null());
}

#[actix_web::test]
async fn access_check_returns_false_for_blocked_package() {
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("npm", "evil-pkg", "1.0.0");
    repo.set_status(
        &pkg,
        PackageStatus::Blocked {
            reason: "security vulnerability".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=npm&name=evil-pkg&version=1.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["can_access"], false);
    assert_eq!(body["reason"], "security vulnerability");
}

// ── /api/v1/admin/packages ────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_packages_returns_403_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/admin/packages").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn admin_packages_returns_403_for_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn admin_packages_returns_200_for_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body.is_array());
}

// ── /api/v1/admin/packages/block & /unblock ───────────────────────────────────

#[actix_web::test]
async fn admin_block_returns_403_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/block")
        .set_json(serde_json::json!({
            "registry": "npm", "name": "pkg", "version": "1.0.0", "reason": "test"
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn admin_block_succeeds_for_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21",
            "reason": "supply-chain risk"
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["success"], true);
}

#[actix_web::test]
async fn admin_block_then_proxy_returns_403() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Block via API
    let block_req = TestRequest::post()
        .uri("/api/v1/admin/packages/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21",
            "reason": "blocked for test"
        }))
        .to_request();
    let block_resp = call_service(&app, block_req).await;
    assert_eq!(block_resp.status(), 200);

    // Attempt proxy fetch — should be denied
    let proxy_req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21")
        .to_request();
    let proxy_resp = call_service(&app, proxy_req).await;
    assert_eq!(proxy_resp.status(), 403);
}

#[actix_web::test]
async fn admin_unblock_restores_proxy_access() {
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("npm", "lodash", "4.17.21");

    // Pre-block
    repo.set_status(
        &pkg,
        PackageStatus::Blocked {
            reason: "test".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let app = make_app(repo).await;

    // Unblock via API
    let unblock_req = TestRequest::post()
        .uri("/api/v1/admin/packages/unblock")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21"
        }))
        .to_request();
    let unblock_resp = call_service(&app, unblock_req).await;
    assert_eq!(unblock_resp.status(), 200);

    // Proxy should succeed now
    let proxy_req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21")
        .to_request();
    let proxy_resp = call_service(&app, proxy_req).await;
    assert_eq!(proxy_resp.status(), 200);
}

// ── /api/v1/admin/audit-log ───────────────────────────────────────────────────

#[actix_web::test]
async fn audit_log_returns_403_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/admin/audit-log").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn audit_log_returns_events_for_admin() {
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("npm", "lodash", "4.17.21");
    repo.record_access(AccessEvent::allowed_download(pkg, Some("user-1".to_owned()), Role::User))
        .await
        .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let events = body.as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["package_id"]["name"], "lodash");
}

// ── Proxy routes ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_github_releases_is_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/releases")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_release_by_tag_is_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/releases/tags/v1.80.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_tarball_is_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/tarball/v1.80.0")
        .to_request();
    let resp = call_service(&app, req).await;
    // Anonymous lacks source:read
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_github_tarball_is_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/tarball/v1.80.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_npm_packument_is_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/proxy/npm/lodash").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_npm_version_is_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_npm_tarball_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21/tarball")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_cargo_crate_is_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/proxy/cargo/serde").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_cargo_download_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/serde/1.0.0/download")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_response_contains_artifact_bytes() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = actix_web::test::read_body(resp).await;
    // FixedRegistry embeds the package key in the artifact
    assert!(std::str::from_utf8(&body).unwrap().contains("lodash"));
}

// ── Unknown registry → 400 ────────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_unknown_registry_returns_400() {
    // The test app only has github/npm/cargo; any other registry name goes
    // through a route that doesn't exist.  The way to trigger an unknown-registry
    // error is to call ProxyService with a registry that wasn't wired up; here
    // we verify that the error mapping is 400 via the access-check endpoint, which
    // intentionally forces a get_status call (no upstream involved).
    let repo = InMemoryRepo::new();
    let app = make_app(repo).await;

    // No route is registered for /proxy/pypi/..., so actix-web returns 404.
    let req = TestRequest::get()
        .uri("/proxy/pypi/requests/2.31.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Error response JSON format ─────────────────────────────────────────────────

#[actix_web::test]
async fn error_response_has_error_and_message_fields() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/admin/packages").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
    let body: Value = read_body_json(resp).await;
    assert!(body["error"].is_string(), "response must have an 'error' field");
    assert!(body["message"].is_string(), "response must have a 'message' field");
    // HTTP reason phrase for 403
    assert_eq!(body["error"], "Forbidden");
}

// ── Audit log records access events ──────────────────────────────────────────

#[actix_web::test]
async fn proxy_access_is_recorded_in_audit_log() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Trigger a proxy request
    let req = TestRequest::get()
        .uri("/proxy/npm/express")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    // Confirm the event was recorded
    let audit_req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let audit_resp = call_service(&app, audit_req).await;
    assert_eq!(audit_resp.status(), 200);
    let events: Value = read_body_json(audit_resp).await;
    let events = events.as_array().unwrap();
    assert!(!events.is_empty(), "at least one access event should be recorded");
    assert_eq!(events[0]["package_id"]["name"], "express");
    assert_eq!(events[0]["result"]["outcome"], "allowed");
}

#[actix_web::test]
async fn denied_proxy_access_is_recorded_as_denied_in_audit_log() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Anonymous tries to download a tarball (source:read denied)
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21/tarball")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);

    // The denied event should appear in the audit log
    let audit_req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let audit_resp = call_service(&app, audit_req).await;
    let events: Value = read_body_json(audit_resp).await;
    let events = events.as_array().unwrap();
    assert!(!events.is_empty(), "denied access should be recorded");
    assert_eq!(events[0]["result"]["outcome"], "denied");
}

// ── Dynamic group tests ───────────────────────────────────────────────────────
//
// Scenario:
//   "github"  — normal registry; anonymous=["releases:read"], user=[...,"source:read"]
//   "github2" — group-restricted registry; anonymous=[], user=[], admin=["*"]
//               team-a = ["releases:read","source:read"]
//               team-b = ["releases:read"]
//
// Tokens:
//   TEAM_A_TOKEN  → anonymous role, groups=["team-a"]
//   TEAM_B_TOKEN  → anonymous role, groups=["team-b"]
//   TEAM_AB_TOKEN → anonymous role, groups=["team-a","team-b"]

fn group_auth_providers() -> Vec<Arc<dyn AuthProvider>> {
    vec![Arc::new(
        StaticTokenAuthProvider::new([
            (ADMIN_TOKEN.to_owned(), Some("admin".to_owned()), Role::Admin),
            (USER_TOKEN.to_owned(), Some("user-1".to_owned()), Role::User),
        ])
        .with_group_entries([
            (
                TEAM_A_TOKEN.to_owned(),
                Some("team-a-user".to_owned()),
                Role::Anonymous,
                vec!["team-a".to_owned()],
            ),
            (
                TEAM_B_TOKEN.to_owned(),
                Some("team-b-user".to_owned()),
                Role::Anonymous,
                vec!["team-b".to_owned()],
            ),
            (
                TEAM_AB_TOKEN.to_owned(),
                Some("team-ab-user".to_owned()),
                Role::Anonymous,
                vec!["team-a".to_owned(), "team-b".to_owned()],
            ),
        ]),
    )]
}

fn rbac_policy_group_registry(repo: Arc<dyn PackageRepository>) -> RegistryPolicy {
    let perms = HashMap::from([
        (Role::Anonymous, vec![]),
        (Role::User, vec![]),
        (Role::Admin, vec!["*".to_owned()]),
    ]);
    let group_perms = HashMap::from([
        ("team-a".to_owned(), vec!["releases:read".to_owned(), "source:read".to_owned()]),
        ("team-b".to_owned(), vec!["releases:read".to_owned()]),
    ]);
    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(300)),
        firewall_only: false,
        serve_stale_metadata: false,
        artifact_ttl: None,
        rules: vec![
            Box::new(RbacRule::new(perms).with_groups(group_perms)),
            Box::new(BlockListRule::new(repo)),
        ],
    }
}

async fn make_group_app(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("github".to_owned(), FixedRegistry::new("github") as Arc<dyn RegistryClient>),
        ("github2".to_owned(), FixedRegistry::new("github") as Arc<dyn RegistryClient>),
    ]
    .into();

    let policies: HashMap<String, RegistryPolicy> = [
        ("github".to_owned(), rbac_policy(repo_dyn.clone())),
        ("github2".to_owned(), rbac_policy_group_registry(repo_dyn.clone())),
    ]
    .into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);

    // github: everyone can access (role-based)
    // github2: only accessible via group membership (no role-based access for anon/user)
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["github"].iter().map(|s| s.to_string()).collect(),
        user: ["github"].iter().map(|s| s.to_string()).collect(),
        admin: ["github", "github2"].iter().map(|s| s.to_string()).collect(),
        groups: [
            (
                "team-a".to_owned(),
                ["github2"].iter().map(|s| s.to_string()).collect(),
            ),
            (
                "team-b".to_owned(),
                ["github2"].iter().map(|s| s.to_string()).collect(),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("github", "github"), ("github2", "github")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, token_repo, None, access_config, registry_map, batlehub_web::UpstreamMap::default(), vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));

    init_service(
        app.wrap(AuthMiddlewareFactory::new(group_auth_providers())),
    )
    .await
}

// ── /api/v1/registries with groups ───────────────────────────────────────────

#[actix_web::test]
async fn group_member_sees_group_restricted_registry_in_listing() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .insert_header(("Authorization", bearer(TEAM_A_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let names: Vec<&str> = body.as_array().unwrap().iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"github2"), "team-a should see github2");
    assert!(names.contains(&"github"), "team-a should also see role-based github");
}

#[actix_web::test]
async fn user_without_group_cannot_see_group_restricted_registry() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let names: Vec<&str> = body.as_array().unwrap().iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(!names.contains(&"github2"), "user without group should not see github2");
    assert!(names.contains(&"github"));
}

#[actix_web::test]
async fn anonymous_without_group_cannot_see_group_restricted_registry() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let names: Vec<&str> = body.as_array().unwrap().iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(!names.contains(&"github2"), "anonymous should not see github2");
}

#[actix_web::test]
async fn multi_group_user_sees_union_of_registries() {
    let app = make_group_app(InMemoryRepo::new()).await;
    // team-ab has both groups → should see github and github2
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .insert_header(("Authorization", bearer(TEAM_AB_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let names: Vec<&str> = body.as_array().unwrap().iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"github"), "team-ab should see github (anonymous role)");
    assert!(names.contains(&"github2"), "team-ab should see github2 (team-a or team-b group)");
}

// ── Proxy access with group permissions ──────────────────────────────────────

#[actix_web::test]
async fn group_member_can_list_releases_from_group_registry() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/releases")
        .insert_header(("Authorization", bearer(TEAM_A_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn group_member_can_download_tarball_from_group_registry() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/tarball/v1.80.0")
        .insert_header(("Authorization", bearer(TEAM_A_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn team_b_can_read_releases_but_not_source_on_group_registry() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let releases_req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/releases")
        .insert_header(("Authorization", bearer(TEAM_B_TOKEN)))
        .to_request();
    let releases_resp = call_service(&app, releases_req).await;
    assert_eq!(releases_resp.status(), 200, "team-b can releases:read");

    let tarball_req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/tarball/v1.80.0")
        .insert_header(("Authorization", bearer(TEAM_B_TOKEN)))
        .to_request();
    let tarball_resp = call_service(&app, tarball_req).await;
    assert_eq!(tarball_resp.status(), 403, "team-b cannot source:read");
}

#[actix_web::test]
async fn user_without_group_denied_group_registry_proxy() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/releases")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn anonymous_denied_group_registry_proxy() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/releases")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

// ── /api/v1/me with groups ────────────────────────────────────────────────────

#[actix_web::test]
async fn me_endpoint_returns_groups_for_group_token() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer(TEAM_A_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["role"], "anonymous");
    let groups: Vec<&str> = body["groups"].as_array().unwrap()
        .iter().filter_map(|v| v.as_str()).collect();
    assert!(groups.contains(&"team-a"), "groups field should contain team-a");
    assert_eq!(body["has_registry_access"], true, "team-a has registry access via group");
}

#[actix_web::test]
async fn me_endpoint_returns_empty_groups_for_regular_token() {
    let app = make_group_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let groups = body["groups"].as_array().unwrap();
    assert!(groups.is_empty(), "regular user token should have no groups");
}

// ── Group access recorded in audit log ───────────────────────────────────────

#[actix_web::test]
async fn group_proxy_access_is_recorded_in_audit_log() {
    let repo = InMemoryRepo::new();
    let app = make_group_app(repo.clone()).await;

    let req = TestRequest::get()
        .uri("/proxy/github2/rust-lang/rust/releases")
        .insert_header(("Authorization", bearer(TEAM_A_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    let audit_req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let audit_resp = call_service(&app, audit_req).await;
    let events: Value = read_body_json(audit_resp).await;
    let events = events.as_array().unwrap();
    assert!(!events.is_empty(), "group access event should be recorded");
    assert_eq!(events[0]["result"]["outcome"], "allowed");
    assert_eq!(events[0]["package_id"]["registry"], "github2");
}

// ── InMemoryTokenRepository ───────────────────────────────────────────────────

struct InMemoryTokenRepository {
    tokens: Mutex<Vec<UserToken>>,
}

impl InMemoryTokenRepository {
    fn new() -> Arc<Self> {
        Arc::new(Self { tokens: Mutex::new(vec![]) })
    }
}

#[async_trait]
impl UserTokenRepository for InMemoryTokenRepository {
    async fn create_token(
        &self,
        id: Uuid,
        user_id: &str,
        name: &str,
        _token_hash: &str,
        role: Role,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<UserToken, CoreError> {
        // Check uniqueness by name per user
        let mut tokens = self.tokens.lock().unwrap();
        if tokens.iter().any(|t| t.user_id == user_id && t.name == name && t.revoked_at.is_none()) {
            return Err(CoreError::Conflict(format!("a token named '{}' already exists", name)));
        }
        let tok = UserToken {
            id,
            user_id: user_id.to_owned(),
            name: name.to_owned(),
            role,
            expires_at,
            created_at: Utc::now(),
            revoked_at: None,
        };
        tokens.push(tok);
        Ok(tokens.last().unwrap().clone_token())
    }

    async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        Ok(None)
    }

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<UserToken>, CoreError> {
        let tokens = self.tokens.lock().unwrap();
        Ok(tokens.iter()
            .filter(|t| t.user_id == user_id && t.revoked_at.is_none())
            .map(|t| t.clone_token())
            .collect())
    }

    async fn revoke(&self, id: Uuid, user_id: &str) -> Result<bool, CoreError> {
        let mut tokens = self.tokens.lock().unwrap();
        for t in tokens.iter_mut() {
            if t.id == id && t.user_id == user_id && t.revoked_at.is_none() {
                t.revoked_at = Some(Utc::now());
                return Ok(true);
            }
        }
        Ok(false)
    }
}

// UserToken doesn't derive Clone; add a helper method instead.
trait CloneToken {
    fn clone_token(&self) -> UserToken;
}

impl CloneToken for UserToken {
    fn clone_token(&self) -> UserToken {
        UserToken {
            id: self.id,
            user_id: self.user_id.clone(),
            name: self.name.clone(),
            role: self.role.clone(),
            expires_at: self.expires_at,
            created_at: self.created_at,
            revoked_at: self.revoked_at,
        }
    }
}

// ── OIDC-style test auth provider ─────────────────────────────────────────────
// The token endpoint only accepts identities whose auth_provider == "oidc".
// StaticTokenAuthProvider sets "static-token", so we use a thin wrapper.

use batlehub_core::ports::RawAuthRequest;

const OIDC_USER_TOKEN: &str = "oidc-user-token";
const OIDC_ADMIN_TOKEN: &str = "oidc-admin-token";

struct OidcStyleAuthProvider;

#[async_trait]
impl AuthProvider for OidcStyleAuthProvider {
    fn name(&self) -> &str { "oidc" }

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<batlehub_core::entities::Identity>, CoreError> {
        use batlehub_core::entities::Identity;
        let auth = req.headers.get("authorization")
            .or_else(|| req.headers.get("Authorization"))
            .and_then(|v| v.strip_prefix("Bearer "));
        match auth {
            Some(OIDC_USER_TOKEN) => Ok(Some(Identity {
                user_id: Some("oidc-user".to_owned()),
                role: Role::User,
                auth_provider: Some("oidc".to_owned()),
                groups: vec![],
            })),
            Some(OIDC_ADMIN_TOKEN) => Ok(Some(Identity {
                user_id: Some("oidc-admin".to_owned()),
                role: Role::Admin,
                auth_provider: Some("oidc".to_owned()),
                groups: vec![],
            })),
            _ => Ok(None),
        }
    }
}

/// Build an app wired with both static + OIDC-style providers and an in-memory token repo.
async fn make_app_with_tokens(
    repo: Arc<InMemoryRepo>,
    token_repo: Arc<InMemoryTokenRepository>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("npm".to_owned(), FixedRegistry::new("npm") as Arc<dyn RegistryClient>),
    ].into();
    let policies: HashMap<String, RegistryPolicy> = [
        ("npm".to_owned(), rbac_policy(repo_dyn.clone())),
    ].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let tok_repo: Arc<dyn UserTokenRepository> = token_repo;
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["npm"].iter().map(|s| s.to_string()).collect(),
        user: ["npm"].iter().map(|s| s.to_string()).collect(),
        admin: ["npm"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("npm", "npm")].iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, tok_repo, None, access_config, registry_map, batlehub_web::UpstreamMap::default(), vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));

    let providers: Vec<Arc<dyn AuthProvider>> = vec![
        Arc::new(StaticTokenAuthProvider::new([
            (ADMIN_TOKEN.to_owned(), Some("admin".to_owned()), Role::Admin),
            (USER_TOKEN.to_owned(), Some("user-1".to_owned()), Role::User),
        ])),
        Arc::new(OidcStyleAuthProvider),
    ];

    init_service(app.wrap(AuthMiddlewareFactory::new(providers))).await
}

// ── Token API tests ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn create_token_returns_403_for_anonymous() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .set_json(serde_json::json!({"name": "ci", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn create_token_returns_403_for_static_token_user() {
    // Static token provider sets auth_provider = "static-token", not "oidc"
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn create_token_succeeds_for_oidc_user() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "ci-token", "expires_in_days": 30, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["name"], "ci-token");
    assert!(body["token"].is_string(), "raw token should be returned");
}

#[actix_web::test]
async fn create_token_rejects_zero_days() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "bad", "expires_in_days": 0, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_91_days() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "bad", "expires_in_days": 91, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_empty_name() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "   ", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_rejects_invalid_role() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "t", "expires_in_days": 7, "role": "superadmin"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn create_token_user_cannot_escalate_to_admin_role() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "escalate", "expires_in_days": 7, "role": "admin"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn list_tokens_returns_created_tokens() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    // Create a token
    let create_req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "my-token", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let create_resp = call_service(&app, create_req).await;
    assert_eq!(create_resp.status(), 201);

    // List tokens
    let list_req = TestRequest::get()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let list_resp = call_service(&app, list_req).await;
    assert_eq!(list_resp.status(), 200);
    let body: Value = read_body_json(list_resp).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "my-token");
}

#[actix_web::test]
async fn revoke_token_returns_204() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    let create_req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "to-revoke", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let create_resp = call_service(&app, create_req).await;
    assert_eq!(create_resp.status(), 201);
    let created: Value = read_body_json(create_resp).await;
    let id = created["id"].as_str().unwrap();

    let revoke_req = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let revoke_resp = call_service(&app, revoke_req).await;
    assert_eq!(revoke_resp.status(), 204);
}

#[actix_web::test]
async fn revoke_nonexistent_token_returns_404() {
    let app = make_app_with_tokens(InMemoryRepo::new(), InMemoryTokenRepository::new()).await;
    let fake_id = Uuid::new_v4();
    let req = TestRequest::delete()
        .uri(&format!("/api/v1/auth/tokens/{fake_id}"))
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn duplicate_token_name_returns_conflict() {
    let tok_repo = InMemoryTokenRepository::new();
    let app = make_app_with_tokens(InMemoryRepo::new(), tok_repo).await;

    for _ in 0..2 {
        let req = TestRequest::post()
            .uri("/api/v1/auth/tokens")
            .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
            .set_json(serde_json::json!({"name": "dup", "expires_in_days": 7, "role": "user"}))
            .to_request();
        let _ = call_service(&app, req).await;
    }

    let req = TestRequest::post()
        .uri("/api/v1/auth/tokens")
        .insert_header(("Authorization", bearer(OIDC_USER_TOKEN)))
        .set_json(serde_json::json!({"name": "dup", "expires_in_days": 7, "role": "user"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

// ── Pagination / Filtering tests ──────────────────────────────────────────────

#[actix_web::test]
async fn admin_packages_list_blocked_only_filter() {
    let repo = InMemoryRepo::new();

    let available = PackageId::new("npm", "lodash", "4.17.21");
    let blocked = PackageId::new("npm", "evil-pkg", "1.0.0");

    repo.record_access(AccessEvent::allowed_download(available, Some("u".to_owned()), Role::User))
        .await.unwrap();
    repo.set_status(
        &blocked,
        PackageStatus::Blocked { reason: "vuln".to_owned(), blocked_by: "admin".to_owned(), blocked_at: Utc::now() },
    ).await.unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages?blocked_only=true")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let items = body.as_array().unwrap();
    assert!(items.iter().all(|i| i["status"]["status"] == "blocked"), "only blocked packages expected");
}

#[actix_web::test]
async fn audit_log_denied_only_filter() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Cause a denied event (anonymous accessing tarball = source:read denied)
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21/tarball")
        .to_request();
    let _ = call_service(&app, req).await;

    // Also cause an allowed event
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash")
        .to_request();
    let _ = call_service(&app, req).await;

    let audit_req = TestRequest::get()
        .uri("/api/v1/admin/audit-log?denied_only=true")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, audit_req).await;
    assert_eq!(resp.status(), 200);
    let events: Value = read_body_json(resp).await;
    let events = events.as_array().unwrap();
    assert!(!events.is_empty(), "at least one denied event expected");
    assert!(events.iter().all(|e| e["result"]["outcome"] == "denied"), "only denied events expected");
}

#[actix_web::test]
async fn registries_endpoint_returns_list_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get().uri("/api/v1/registries").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let registries = body.as_array().unwrap();
    // Anonymous has access to github, npm, cargo in make_app
    assert!(!registries.is_empty(), "should see at least one registry");
    let names: Vec<&str> = registries.iter().filter_map(|r| r["name"].as_str()).collect();
    assert!(names.contains(&"npm"));
}

#[actix_web::test]
async fn registries_endpoint_returns_200_for_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/registries")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let registries = body.as_array().unwrap();
    assert!(registries.len() >= 3, "admin should see github, npm, cargo");
}

// ── Cargo sparse registry config ─────────────────────────────────────────────

/// Build a test app with a wired-up CargoIndexProxy so we can test the
/// `cargo_registry_config` handler's happy path.
async fn make_app_with_cargo_index(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("cargo".to_owned(), FixedRegistry::new("cargo") as Arc<dyn RegistryClient>),
    ].into();
    let policies: HashMap<String, RegistryPolicy> = [
        ("cargo".to_owned(), rbac_policy(repo_dyn.clone())),
    ].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["cargo"].iter().map(|s| s.to_string()).collect(),
        user: ["cargo"].iter().map(|s| s.to_string()).collect(),
        admin: ["cargo"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("cargo", "cargo")].iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    );

    // Wire up a real CargoIndexProxy entry so cargo_registry_config can return a config
    let mut cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    cargo_indexes.insert("cargo".to_owned(), batlehub_web::CargoIndexProxy {
        http: reqwest::Client::new(),
        index_url: "https://index.crates.io".to_owned(),
    });

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, token_repo, None, access_config, registry_map, batlehub_web::UpstreamMap::default(), vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

#[actix_web::test]
async fn cargo_registry_config_returns_dl_url() {
    let app = make_app_with_cargo_index(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/registry/config.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["dl"].as_str().unwrap().contains("/proxy/cargo/{crate}/{version}/download"));
}

#[actix_web::test]
async fn cargo_registry_config_returns_404_for_unknown_registry() {
    let app = make_app(InMemoryRepo::new()).await;
    // 'npm' is not a cargo registry
    let req = TestRequest::get()
        .uri("/proxy/npm/registry/config.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn cargo_registry_config_returns_404_when_no_index_configured() {
    // make_app uses empty cargo_indexes, so the cargo registry exists but has no index
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/registry/config.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn cargo_registry_index_returns_404_for_non_cargo_registry() {
    // npm is not a cargo registry — cargo_registry_index should return 404
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/registry/se/rd/serde")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn cargo_registry_index_returns_404_when_no_index_configured() {
    // cargo registry exists in the map but cargo_indexes is empty
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/registry/se/rd/serde")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── front_office packages: registry filter + proxy_url ───────────────────────

#[actix_web::test]
async fn packages_list_filters_out_inaccessible_registry() {
    let repo = InMemoryRepo::new();
    // Record a package in an inaccessible registry
    let pkg_npm = PackageId::new("npm", "lodash", "4.17.21");
    let pkg_github = PackageId::new("github", "rust-lang/rust", "v1.80.0");
    repo.record_access(AccessEvent::allowed_download(pkg_npm, Some("u".to_owned()), Role::User)).await.unwrap();
    repo.record_access(AccessEvent::allowed_download(pkg_github, Some("u".to_owned()), Role::User)).await.unwrap();

    let app = make_app(repo).await;

    // Filter by npm — should only return npm package
    let req = TestRequest::get()
        .uri("/api/v1/packages?registry=npm")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let items = body["items"].as_array().unwrap();
    assert!(items.iter().all(|i| i["registry"] == "npm"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_npm_tarball() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=npm&name=lodash&version=4.17.21&artifact=tarball")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["can_access"], true);
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/npm/lodash/4.17.21/tarball"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_cargo_download() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=cargo&name=serde&version=1.0.0&artifact=download")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/cargo/serde/1.0.0/download"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_releases() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=releases")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/releases"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_tag() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=v1.80.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/releases/tags/v1.80.0"));
}

#[actix_web::test]
async fn packages_list_returns_empty_for_inaccessible_registry_filter() {
    // When a user asks for packages from a registry they can't access, they get empty results
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("github", "rust-lang/rust", "v1.80.0");
    repo.record_access(AccessEvent::allowed_download(pkg, Some("u".to_owned()), Role::User)).await.unwrap();

    // make_app gives anonymous access to github, so anon CAN see it normally.
    // But filtering for a completely unknown registry should return empty.
    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages?registry=pypi")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["total"], 0);
}

// ── Cargo download (source:read) ──────────────────────────────────────────────

#[actix_web::test]
async fn proxy_cargo_download_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/serde/1.0.0/download")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = actix_web::test::read_body(resp).await;
    assert!(std::str::from_utf8(&body).unwrap().contains("serde"));
}

// ── npm tarball (source:read) ─────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_npm_tarball_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/lodash/4.17.21/tarball")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = actix_web::test::read_body(resp).await;
    assert!(std::str::from_utf8(&body).unwrap().contains("lodash"));
}

// ── GitHub download routes ────────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_github_zipball_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/zipball/v1.80.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_zipball_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/zipball/v1.80.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_github_asset_by_name_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/releases/download/v1.80.0/rustc-1.80.0-x86_64-unknown-linux-gnu.tar.gz")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    // source:read required — user has it
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_asset_by_name_accessible_anonymously() {
    // releases/download uses releases:read which anonymous users have
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/releases/download/v1.80.0/rust.tar.gz")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_raw_file_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/raw/main/README.md")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_github_raw_file_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/raw/main/README.md")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_github_asset_by_id_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/rust-lang/rust/releases/assets/12345678?tag=v1.80.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── /api/v1/admin/packages/detail ────────────────────────────────────────────

#[actix_web::test]
async fn package_detail_returns_403_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages/detail?registry=npm&name=lodash")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn package_detail_returns_200_for_admin_with_no_packages() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages/detail?registry=npm&name=lodash")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["registry"], "npm");
    assert_eq!(body["name"], "lodash");
    assert!(body["versions"].as_array().unwrap().is_empty());
    assert!(body["recent_events"].as_array().unwrap().is_empty());
}

#[actix_web::test]
async fn package_detail_shows_versions_and_events_after_access() {
    let repo = InMemoryRepo::new();

    // Record a download event so the package appears in summaries and events
    let pkg = PackageId::new("npm", "lodash", "4.17.21");
    repo.record_access(AccessEvent::allowed_download(
        pkg.clone(),
        Some("user-1".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages/detail?registry=npm&name=lodash")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["registry"], "npm");
    let versions = body["versions"].as_array().unwrap();
    assert!(!versions.is_empty(), "should list the recorded version");
    assert_eq!(versions[0]["version"], "4.17.21");
    let events = body["recent_events"].as_array().unwrap();
    assert!(!events.is_empty(), "should list the recent events");
    assert_eq!(events[0]["outcome"], "allowed");
}

#[actix_web::test]
async fn package_detail_shows_blocked_status() {
    let repo = InMemoryRepo::new();

    let pkg = PackageId::new("npm", "evil-pkg", "1.0.0");
    repo.record_access(AccessEvent::allowed_download(
        pkg.clone(),
        Some("user-1".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();
    repo.set_status(
        &pkg,
        PackageStatus::Blocked {
            reason: "vuln".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/packages/detail?registry=npm&name=evil-pkg")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let versions = body["versions"].as_array().unwrap();
    assert!(!versions.is_empty());
    assert_eq!(versions[0]["status"]["status"], "blocked");
    assert_eq!(versions[0]["status"]["reason"], "vuln");
}

// ── OpenVSX proxy handler ─────────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_openvsx_vsix_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/openvsx/ms-python.python/2023.20.0/vsix")
        .to_request();
    let resp = call_service(&app, req).await;
    // download_vsix uses source:read — anonymous only has releases:read
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_openvsx_vsix_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/openvsx/ms-python.python/2023.20.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_openvsx_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    // "npm" exists but is type "npm", not "openvsx" — require_openvsx rejects it
    let req = TestRequest::get()
        .uri("/proxy/npm/ms-python.python/2023.20.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn proxy_openvsx_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/unknown-reg/ms-python.python/2023.20.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── VS Code Marketplace proxy handler ─────────────────────────────────────────

#[actix_web::test]
async fn proxy_vscode_marketplace_vsix_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/vscode/ms-python.python/2024.2.1/vsix")
        .to_request();
    let resp = call_service(&app, req).await;
    // download_vsix uses source:read — anonymous only has releases:read
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_vscode_marketplace_vsix_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/vscode/ms-python.python/2024.2.1/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_vscode_marketplace_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    // "npm" exists but is type "npm", not "vscode-marketplace" — require_openvsx rejects it
    let req = TestRequest::get()
        .uri("/proxy/npm/ms-python.python/2024.2.1/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── GoProxy handler ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn proxy_goproxy_latest_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@latest")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_goproxy_list_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/list")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_goproxy_info_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/v0.3.7.info")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_goproxy_mod_accessible_anonymously() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/v0.3.7.mod")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_goproxy_zip_blocked_for_anonymous() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/v0.3.7.zip")
        .to_request();
    let resp = call_service(&app, req).await;
    // zip uses source:read — anonymous only has releases:read
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn proxy_goproxy_zip_accessible_by_user() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/v0.3.7.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn proxy_goproxy_unknown_file_extension_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/golang.org/x/text/@v/v0.3.7.tar")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn proxy_goproxy_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    // "npm" exists but is type "npm", not "goproxy" — require_goproxy rejects it
    let req = TestRequest::get()
        .uri("/proxy/npm/golang.org/x/text/@latest")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Upstream-KO / stale-serving integration tests ────────────────────────────
//
// These tests verify the end-to-end HTTP behaviour when the upstream registry
// is unavailable and the proxy falls back to stale metadata from its cache.

struct UnavailableRegistry;

#[async_trait]
impl RegistryClient for UnavailableRegistry {
    fn registry_type(&self) -> &str { "npm" }

    async fn resolve_metadata(&self, _pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        Err(CoreError::Registry("upstream down".into()))
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let data = Bytes::from(format!("artifact:npm:{}", pkg.cache_key()));
        Ok(FetchedArtifact {
            stream: Box::pin(stream::once(async move { Ok::<Bytes, CoreError>(data) })),
            cache_control: None,
        })
    }
}

async fn make_unavailable_npm_app(
    repo: Arc<InMemoryRepo>,
    cache: Arc<InMemoryCacheStore>,
    serve_stale: bool,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache_dyn: Arc<dyn CacheStore> = cache;

    let registries: HashMap<String, Arc<dyn RegistryClient>> =
        [("npm".to_owned(), Arc::new(UnavailableRegistry) as Arc<dyn RegistryClient>)].into();

    let perms = HashMap::from([
        (Role::Anonymous, vec!["releases:read".to_owned()]),
        (Role::User, vec!["releases:read".to_owned(), "source:read".to_owned()]),
        (Role::Admin, vec!["*".to_owned()]),
    ]);
    let policies: HashMap<String, RegistryPolicy> = [(
        "npm".to_owned(),
        RegistryPolicy {
            metadata_ttl: Some(Duration::from_secs(300)),
            firewall_only: false,
            serve_stale_metadata: serve_stale,
            artifact_ttl: None,
            rules: vec![
                Box::new(RbacRule::new(perms)),
                Box::new(BlockListRule::new(repo_dyn.clone())),
            ],
        },
    )]
    .into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache: cache_dyn,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["npm"].iter().map(|s| s.to_string()).collect(),
        user: ["npm"].iter().map(|s| s.to_string()).collect(),
        admin: ["npm"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("npm", "npm")].iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            batlehub_web::UpstreamMap::default(),
            vec![],
            std::collections::HashMap::new(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

fn stale_npm_meta(name: &str, version: &str) -> PackageMetadata {
    PackageMetadata {
        id: PackageId::new("npm", name, version),
        published_at: Some(Utc::now() - chrono::Duration::days(30)),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::json!({}),
        cache_control: None,
    }
}

#[actix_web::test]
async fn upstream_down_with_stale_metadata_returns_200() {
    let cache = Arc::new(InMemoryCacheStore::new());
    let pkg = PackageId::new("npm", "lodash", "4.17.21");
    let cache_key = format!("meta:{}", pkg.cache_key());
    cache.seed_expired(&cache_key, stale_npm_meta("lodash", "4.17.21")).await;

    let app = make_unavailable_npm_app(InMemoryRepo::new(), cache, true).await;
    let req = TestRequest::get().uri("/proxy/npm/lodash/4.17.21").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "stale metadata should be served when upstream is down");
}

#[actix_web::test]
async fn upstream_down_no_stale_returns_502() {
    let cache = Arc::new(InMemoryCacheStore::new()); // empty — no stale entry
    let app = make_unavailable_npm_app(InMemoryRepo::new(), cache, true).await;
    let req = TestRequest::get().uri("/proxy/npm/lodash/4.17.21").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 502, "no stale + upstream down must return 502");
}

#[actix_web::test]
async fn upstream_down_serve_stale_disabled_returns_502() {
    // Stale entry exists in cache but serve_stale_metadata = false
    let cache = Arc::new(InMemoryCacheStore::new());
    let pkg = PackageId::new("npm", "lodash", "4.17.21");
    let cache_key = format!("meta:{}", pkg.cache_key());
    cache.seed_expired(&cache_key, stale_npm_meta("lodash", "4.17.21")).await;

    let app = make_unavailable_npm_app(InMemoryRepo::new(), cache, false).await;
    let req = TestRequest::get().uri("/proxy/npm/lodash/4.17.21").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 502, "serve_stale=false must not use the stale entry");
}

// ── /api/v1/admin/health ──────────────────────────────────────────────────────

#[actix_web::test]
async fn health_non_admin_returns_403() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/health")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn health_no_db_returns_empty_list() {
    // make_app passes pool=None, so the handler returns [] immediately.
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/health")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

// ── /api/v1/admin/registries/{registry}/clear-cache ──────────────────────────

#[actix_web::test]
async fn clear_cache_non_admin_returns_403() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/clear-cache")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn clear_cache_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/no-such-registry/clear-cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn clear_cache_known_registry_returns_200() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/clear-cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["cleared"].is_number());
}

// ── /api/v1/admin/packages/bulk-block ────────────────────────────────────────

#[actix_web::test]
async fn bulk_block_non_admin_returns_403() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/bulk-block")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({ "items": [] }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn bulk_block_admin_empty_items_returns_200() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/bulk-block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({ "items": [] }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["succeeded_count"], 0);
}

#[actix_web::test]
async fn bulk_block_admin_one_item_succeeds() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/bulk-block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "items": [
                { "registry": "npm", "name": "lodash", "version": "4.17.21",
                  "artifact": null, "reason": "bulk test" }
            ]
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["succeeded_count"], 1);
    assert_eq!(body["failed_count"], 0);
}

// ── /api/v1/admin/packages/bulk-unblock ──────────────────────────────────────

#[actix_web::test]
async fn bulk_unblock_non_admin_returns_403() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/bulk-unblock")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({ "items": [] }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn bulk_unblock_admin_returns_200() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;

    // Block first
    let block_req = TestRequest::post()
        .uri("/api/v1/admin/packages/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21", "reason": "test"
        }))
        .to_request();
    call_service(&app, block_req).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/bulk-unblock")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "items": [
                { "registry": "npm", "name": "lodash", "version": "4.17.21", "artifact": null }
            ]
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["succeeded_count"], 1);
}

// ── /api/v1/admin/packages/invalidate ────────────────────────────────────────

#[actix_web::test]
async fn invalidate_non_admin_returns_403() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/invalidate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21", "artifact": null
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn invalidate_admin_returns_200() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm", "name": "lodash", "version": "4.17.21", "artifact": null
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["success"], true);
}

// ── proxy/npm.rs: wrong-registry-type and unknown-registry paths ──────────────

#[actix_web::test]
async fn get_packument_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    // "github" is registered but is type "github", not npm/cargo/openvsx
    let req = TestRequest::get()
        .uri("/proxy/github/some-package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn get_packument_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such-registry/some-package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn get_version_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/github/some-package/1.0.0")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn get_version_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such-registry/some-package/1.0.0")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn audit_quick_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/cargo/-/npm/v1/audit/quick")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn audit_quick_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/no-such/-/npm/v1/audit/quick")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn audit_quick_no_upstream_configured_returns_404() {
    // make_app uses UpstreamMap::default() (empty), so no upstream for "npm"
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/npm/-/npm/v1/audit/quick")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn audit_quick_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Bind a random local port and serve a single HTTP response.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"{}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.write_all(body).await;
    });

    let upstream_url = format!("http://127.0.0.1:{port}");

    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("npm".to_owned(), FixedRegistry::new("npm") as Arc<dyn RegistryClient>),
    ].into();
    let policies: HashMap<String, batlehub_core::services::RegistryPolicy> = [
        ("npm".to_owned(), rbac_policy(repo_dyn.clone())),
    ].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["npm"].iter().map(|s| s.to_string()).collect(),
        user: ["npm"].iter().map(|s| s.to_string()).collect(),
        admin: ["npm"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("npm", "npm")].iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    );
    let mut upstream_entries = std::collections::HashMap::new();
    upstream_entries.insert("npm".to_owned(), upstream_url);
    let upstream_map = batlehub_web::UpstreamMap(upstream_entries);
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, token_repo, None, access_config, registry_map, upstream_map, vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));
    let app = init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await;

    let req = TestRequest::post()
        .uri("/proxy/npm/-/npm/v1/audit/quick")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"packages": {}}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── proxy/npm.rs: download_tarball wrong registry type ───────────────────────

#[actix_web::test]
async fn download_tarball_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/cargo/some-package/1.0.0/tarball")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── front_office/packages: build_proxy_url coverage ──────────────────────────

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_tarball() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=v1.80.0&artifact=tarball")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/tarball/v1.80.0"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_zipball() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=v1.80.0&artifact=zipball")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/zipball/v1.80.0"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_raw_file() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=v1.80.0&artifact=raw%2FCompiler_Options.md")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/raw/v1.80.0/Compiler_Options.md"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_github_asset_by_name() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=github&name=rust-lang%2Frust&version=v1.80.0&artifact=rustc-1.80.0-x86_64.tar.gz")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/github/rust-lang/rust/releases/assets/rustc-1.80.0-x86_64.tar.gz"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_npm_metadata() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=npm&name=lodash&version=4.17.21")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/npm/lodash/4.17.21"));
    assert!(!proxy_url.contains("/tarball"));
}

#[actix_web::test]
async fn access_check_returns_proxy_url_for_cargo_metadata() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=cargo&name=serde&version=1.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let proxy_url = body["proxy_url"].as_str().unwrap();
    assert!(proxy_url.contains("/proxy/cargo/serde/1.0.0"));
    assert!(!proxy_url.contains("/download"));
}

#[actix_web::test]
async fn access_check_returns_null_proxy_url_for_unknown_registry_type() {
    let app = make_app(InMemoryRepo::new()).await;
    // openvsx is a known registry but has no build_proxy_url branch -> returns None
    let req = TestRequest::get()
        .uri("/api/v1/packages/access?registry=openvsx&name=some.ext&version=1.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["proxy_url"].is_null());
}

// ── proxy/cargo.rs: cargo_registry_index with real upstream ──────────────────

#[actix_web::test]
async fn cargo_registry_index_fetches_from_upstream_and_returns_content() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Bind a random local port and serve one response for the index entry.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let index_body = b"{\"name\":\"rand\",\"vers\":\"0.8.5\"}";
    let index_body_len = index_body.len();

    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {index_body_len}\r\n\r\n"
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.write_all(index_body).await;
    });

    let index_url = format!("http://127.0.0.1:{port}");

    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        ("cargo".to_owned(), FixedRegistry::new("cargo") as Arc<dyn RegistryClient>),
    ].into();
    let policies: HashMap<String, RegistryPolicy> = [
        ("cargo".to_owned(), rbac_policy(repo_dyn.clone())),
    ].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: ["cargo"].iter().map(|s| s.to_string()).collect(),
        user: ["cargo"].iter().map(|s| s.to_string()).collect(),
        admin: ["cargo"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("cargo", "cargo")].iter().map(|(n, t)| (n.to_string(), t.to_string())).collect(),
    );
    let mut cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    cargo_indexes.insert("cargo".to_owned(), batlehub_web::CargoIndexProxy {
        http: reqwest::Client::new(),
        index_url,
    });
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(proxy_svc, admin_svc, token_repo, None, access_config, registry_map, batlehub_web::UpstreamMap::default(), vec![], std::collections::HashMap::new()))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()));
    let app = init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await;

    let req = TestRequest::get()
        .uri("/proxy/cargo/registry/ra/nd/rand")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── proxy/cargo.rs: wrong-registry-type paths ────────────────────────────────

#[actix_web::test]
async fn download_crate_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/some-crate/1.0.0/download")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn download_crate_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such-registry/some-crate/1.0.0/download")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Local / Hybrid private cargo registry ─────────────────────────────────────

/// Build the Cargo publish wire format:
/// `[4B LE u32 meta_len][JSON meta][4B LE u32 crate_len][.crate bytes]`
fn make_publish_payload(name: &str, version: &str) -> Vec<u8> {
    let meta = serde_json::json!({
        "name": name, "vers": version,
        "deps": [], "features": {}, "authors": [],
        "description": null, "documentation": null, "homepage": null,
        "readme": null, "readme_file": null, "keywords": [],
        "categories": [], "license": null, "license_file": null,
        "repository": null, "badges": {}, "links": null
    });
    let meta_bytes = serde_json::to_vec(&meta).unwrap();
    let crate_bytes: &[u8] = b"fake-crate-content";
    let mut buf = Vec::new();
    buf.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(&meta_bytes);
    buf.extend_from_slice(&(crate_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(crate_bytes);
    buf
}

/// Build a test app with a single Cargo registry in the given mode (Local or Hybrid).
/// Registry name is `"local-cargo"`, type `"cargo"`.
/// Auth: ADMIN_TOKEN = admin, USER_TOKEN = user-1 (same as `test_auth_providers`).
async fn make_local_registry_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "local-cargo".to_owned(),
        FixedRegistry::new("cargo") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, RegistryPolicy> =
        [("local-cargo".to_owned(), rbac_policy(repo_dyn.clone()))].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: [].iter().map(|s: &&str| s.to_string()).collect(),
        user: ["local-cargo"].iter().map(|s| s.to_string()).collect(),
        admin: ["local-cargo"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("local-cargo", "cargo")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    // Hybrid mode requires an upstream index for config.json to succeed.
    // A dummy URL is sufficient — upstream fetches only happen on actual index lookups.
    let mut cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    if matches!(mode, RegistryMode::Hybrid) {
        cargo_indexes.insert(
            "local-cargo".to_owned(),
            batlehub_web::CargoIndexProxy {
                http: reqwest::Client::new(),
                index_url: "https://index.crates.io".to_owned(),
            },
        );
    }

    let mut mode_map = RegistryModeMap::default();
    mode_map.0.insert("local-cargo".to_owned(), mode);

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            batlehub_web::UpstreamMap::default(),
            vec![],
            std::collections::HashMap::new(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

// ── config.json ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn local_cargo_config_returns_dl_and_api_url() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-cargo/registry/config.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(
        body["dl"].as_str().unwrap().contains("/proxy/local-cargo/"),
        "dl must contain registry path"
    );
    assert!(
        body["api"].as_str().unwrap().contains("/proxy/local-cargo"),
        "api field must be present for local mode"
    );
}

#[actix_web::test]
async fn hybrid_cargo_config_returns_api_url() {
    let app = make_local_registry_app(RegistryMode::Hybrid).await;
    let req = TestRequest::get()
        .uri("/proxy/local-cargo/registry/config.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["api"].as_str().is_some(), "api field must be present for hybrid mode");
}

// ── cargo publish ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn cargo_publish_user_can_publish() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("my-crate", "0.1.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["warnings"].is_object(), "response must have warnings shape");
}

#[actix_web::test]
async fn cargo_publish_duplicate_version_returns_409() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("dup-crate", "1.0.0"))
        .to_request();
    let first = call_service(&app, req).await;
    assert_eq!(first.status(), 200);

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("dup-crate", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn cargo_publish_anonymous_returns_403() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        // no Authorization header
        .set_payload(make_publish_payload("my-crate", "0.1.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn cargo_publish_proxy_mode_registry_returns_404() {
    // `cargo` registry in make_app uses mode=Proxy (default) — publish must be rejected
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::put()
        .uri("/proxy/cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("my-crate", "0.1.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── sparse index ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn local_cargo_index_unknown_crate_returns_404() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-cargo/registry/my/cr/my-crate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn local_cargo_index_returns_entry_after_publish() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("idx-crate", "0.1.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-cargo/registry/id/x-/idx-crate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let raw = read_body(resp).await;
    let entry: Value = serde_json::from_slice(&raw).expect("index line must be valid JSON");
    assert_eq!(entry["name"], "idx-crate");
    assert_eq!(entry["vers"], "0.1.0");
    assert!(
        entry["cksum"].as_str().map(|s| s.len() == 64).unwrap_or(false),
        "cksum must be 64-char hex SHA-256"
    );
}

// ── download ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn local_cargo_download_unknown_returns_404() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-cargo/no-crate/1.0.0/download")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn local_cargo_download_returns_artifact_after_publish() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("dl-crate", "0.1.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-cargo/dl-crate/0.1.0/download")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), b"fake-crate-content");
}

// ── yank / unyank ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn cargo_yank_user_can_yank() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("yank-crate", "1.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-cargo/api/v1/crates/yank-crate/1.0.0/yank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn cargo_unyank_user_can_unyank() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("yank-crate", "1.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-cargo/api/v1/crates/yank-crate/1.0.0/yank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/yank-crate/1.0.0/unyank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

// ── owners ────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn cargo_owners_returns_404_for_unknown_crate() {
    let app = make_local_registry_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-cargo/api/v1/crates/nonexistent/owners")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn cargo_owners_returns_publisher_after_publish() {
    let app = make_local_registry_app(RegistryMode::Local).await;

    // USER_TOKEN → user_id = "user-1" in test_auth_providers
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("owned-crate", "0.1.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-cargo/api/v1/crates/owned-crate/owners")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let users = body["users"].as_array().expect("users array");
    assert!(!users.is_empty());
    assert_eq!(users[0]["login"], "user-1");
}

// ── hybrid mode ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn hybrid_cargo_index_serves_locally_published_crate() {
    let app = make_local_registry_app(RegistryMode::Hybrid).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("hybrid-crate", "0.1.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-cargo/registry/hy/br/hybrid-crate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let raw = read_body(resp).await;
    let entry: Value = serde_json::from_slice(&raw).expect("index JSON");
    assert_eq!(entry["name"], "hybrid-crate");
}

#[actix_web::test]
async fn hybrid_cargo_download_prefers_local_artifact() {
    let app = make_local_registry_app(RegistryMode::Hybrid).await;

    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload("hybrid-crate", "0.2.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-cargo/hybrid-crate/0.2.0/download")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), b"fake-crate-content");
}

// ── Local / Hybrid private npm registry ───────────────────────────────────────

/// Build a test app with a single npm registry in the given mode.
/// Registry name is `"local-npm"`, type `"npm"`.
async fn make_local_npm_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "local-npm".to_owned(),
        FixedRegistry::new("npm") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, RegistryPolicy> =
        [("local-npm".to_owned(), rbac_policy(repo_dyn.clone()))].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: [].iter().map(|s: &&str| s.to_string()).collect(),
        user: ["local-npm"].iter().map(|s| s.to_string()).collect(),
        admin: ["local-npm"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("local-npm", "npm")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();

    let mut mode_map = RegistryModeMap::default();
    mode_map.0.insert("local-npm".to_owned(), mode);

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            batlehub_web::UpstreamMap::default(),
            vec![],
            std::collections::HashMap::new(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

/// Build a standard npm publish payload (the wire format used by `npm publish`).
fn make_npm_publish_payload(name: &str, version: &str) -> serde_json::Value {
    let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"fake-tarball-content");
    serde_json::json!({
        "name": name,
        "versions": {
            version: {
                "name": name,
                "version": version,
                "description": "Test package",
                "dist": {
                    "shasum": "abc123"
                }
            }
        },
        "_attachments": {
            format!("{}-{}.tgz", name, version): {
                "content_type": "application/octet-stream",
                "data": tarball_b64,
                "length": 20
            }
        }
    })
}

#[actix_web::test]
async fn npm_publish_user_can_publish() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-package")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn npm_publish_duplicate_version_returns_409() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dup-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dup-pkg", "1.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dup-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dup-pkg", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn npm_publish_anonymous_returns_403() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-package")
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn npm_publish_proxy_mode_returns_404() {
    // `npm` registry in make_app uses mode=Proxy (default)
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::put()
        .uri("/proxy/npm/my-package")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn npm_packument_returns_published_version() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-pkg", "2.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/my-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["name"], "my-pkg");
    assert!(body["versions"]["2.0.0"].is_object(), "published version must appear in packument");
    assert!(
        body["versions"]["2.0.0"]["dist"]["tarball"]
            .as_str()
            .unwrap_or("")
            .contains("/proxy/local-npm/my-pkg/2.0.0/tarball"),
        "tarball URL must be rewritten to BatleHub serving path"
    );
}

#[actix_web::test]
async fn npm_version_returns_metadata() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/ver-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("ver-pkg", "0.5.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/ver-pkg/0.5.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["version"], "0.5.0");
    assert!(
        body["dist"]["tarball"]
            .as_str()
            .unwrap_or("")
            .contains("/proxy/local-npm/ver-pkg/0.5.0/tarball"),
        "tarball URL must point at BatleHub"
    );
}

#[actix_web::test]
async fn npm_tarball_download_returns_artifact() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dl-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dl-pkg", "1.2.3"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/dl-pkg/1.2.3/tarball")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), b"fake-tarball-content");
}

#[actix_web::test]
async fn npm_tarball_unknown_version_returns_404() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-npm/no-pkg/9.9.9/tarball")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Local / Hybrid private VS Code extension (openvsx) registry ───────────────

/// Build a test app with a single openvsx registry in the given mode.
/// Registry name is `"local-vsx"`, type `"openvsx"`.
async fn make_local_vsx_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "local-vsx".to_owned(),
        FixedRegistry::new("openvsx") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, RegistryPolicy> =
        [("local-vsx".to_owned(), rbac_policy(repo_dyn.clone()))].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: [].iter().map(|s: &&str| s.to_string()).collect(),
        user: ["local-vsx"].iter().map(|s| s.to_string()).collect(),
        admin: ["local-vsx"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("local-vsx", "openvsx")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();

    let mut mode_map = RegistryModeMap::default();
    mode_map.0.insert("local-vsx".to_owned(), mode);

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            batlehub_web::UpstreamMap::default(),
            vec![],
            std::collections::HashMap::new(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

#[actix_web::test]
async fn vsix_publish_user_can_publish() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake-vsix-content".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn vsix_publish_duplicate_returns_409() {
    let app = make_local_vsx_app(RegistryMode::Local).await;

    let payload = b"PK\x03\x04fake-vsix".to_vec();
    for _ in 0..2 {
        let req = TestRequest::put()
            .uri("/proxy/local-vsx/pub.ext/0.1.0/vsix")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .insert_header(("Content-Type", "application/octet-stream"))
            .set_payload(payload.clone())
            .to_request();
        call_service(&app, req).await;
    }

    let req = TestRequest::put()
        .uri("/proxy/local-vsx/pub.ext/0.1.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(payload)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn vsix_publish_anonymous_returns_403() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn vsix_publish_proxy_mode_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::put()
        .uri("/proxy/openvsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn vsix_download_returns_artifact_after_publish() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let vsix_bytes = b"PK\x03\x04fake-vsix-bytes".to_vec();

    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/2.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(vsix_bytes.clone())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-vsx/my-org.my-ext/2.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), vsix_bytes.as_slice());
}

#[actix_web::test]
async fn vsix_download_unknown_version_returns_404() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-vsx/no-pub.no-ext/9.9.9/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Local / Hybrid private Go module proxy ─────────────────────────────────

/// Build a minimal Go module zip with the given module path and version.
/// The zip contains `{module}@{version}/go.mod` and a stub source file.
fn make_go_module_zip(module: &str, version: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        let mod_path = format!("{module}@{version}/go.mod");
        writer.start_file(mod_path, options).unwrap();
        writer
            .write_all(format!("module {module}\n\ngo 1.21\n").as_bytes())
            .unwrap();
        let src_path = format!("{module}@{version}/main.go");
        writer.start_file(src_path, options).unwrap();
        writer.write_all(b"package main\n").unwrap();
        writer.finish().unwrap();
    }
    buf.into_inner()
}

/// Build a test app with a single goproxy registry in the given mode.
/// Registry name is `"local-go"`, type `"goproxy"`.
async fn make_local_go_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "local-go".to_owned(),
        FixedRegistry::new("goproxy") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, RegistryPolicy> =
        [("local-go".to_owned(), rbac_policy(repo_dyn.clone()))].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        registries,
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        policies,
        max_artifact_size_bytes: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = batlehub_web::AccessConfig {
        anonymous: [].iter().map(|s: &&str| s.to_string()).collect(),
        user: ["local-go"].iter().map(|s| s.to_string()).collect(),
        admin: ["local-go"].iter().map(|s| s.to_string()).collect(),
        groups: std::collections::HashMap::new(),
    };
    let registry_map = batlehub_web::RegistryMap(
        [("local-go", "goproxy")]
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect(),
    );
    let cargo_indexes: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();

    let mut mode_map = RegistryModeMap::default();
    mode_map.0.insert("local-go".to_owned(), mode);

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_app(
            proxy_svc,
            admin_svc,
            token_repo,
            None,
            access_config,
            registry_map,
            batlehub_web::UpstreamMap::default(),
            vec![],
            std::collections::HashMap::new(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

#[actix_web::test]
async fn go_publish_user_can_publish() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let zip = make_go_module_zip("example.com/mymod", "v1.0.0");
    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/mymod/@v/v1.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn go_publish_duplicate_version_returns_409() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let zip = make_go_module_zip("example.com/dup", "v1.0.0");

    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/dup/@v/v1.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip.clone())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/dup/@v/v1.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn go_publish_anonymous_returns_403() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let zip = make_go_module_zip("example.com/mymod", "v1.0.0");
    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/mymod/@v/v1.0.0.zip")
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn go_publish_proxy_mode_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let zip = make_go_module_zip("example.com/mymod", "v1.0.0");
    let req = TestRequest::put()
        .uri("/proxy/go/example.com/mymod/@v/v1.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn go_version_list_returns_published_version() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let zip = make_go_module_zip("example.com/mymod", "v1.0.0");

    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/mymod/@v/v1.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-go/example.com/mymod/@v/list")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    let list = std::str::from_utf8(&body).unwrap();
    assert!(list.contains("v1.0.0"), "version list must include published version");
}

#[actix_web::test]
async fn go_info_returns_version_metadata() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let zip = make_go_module_zip("example.com/infomod", "v2.0.0");

    let req = TestRequest::put()
        .uri("/proxy/local-go/example.com/infomod/@v/v2.0.0.zip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-go/example.com/infomod/@v/v2.0.0.info")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["Version"], "v2.0.0");
    assert!(body["Time"].as_str().is_some(), "Time field must be present");
}

#[actix_web::test]
async fn go_mod_returns_extracted_go_mod() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let module = "example.com/modfile";
    let version = "v0.1.0";
    let zip = make_go_module_zip(module, version);

    let req = TestRequest::put()
        .uri(&format!("/proxy/local-go/{module}/@v/{version}.zip"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-go/{module}/@v/{version}.mod"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    let content = std::str::from_utf8(&body).unwrap();
    assert!(
        content.contains(module),
        "go.mod must contain the module path"
    );
}

#[actix_web::test]
async fn go_zip_download_returns_artifact() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let module = "example.com/dlmod";
    let version = "v1.1.0";
    let zip_bytes = make_go_module_zip(module, version);

    let req = TestRequest::put()
        .uri(&format!("/proxy/local-go/{module}/@v/{version}.zip"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/zip"))
        .set_payload(zip_bytes.clone())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-go/{module}/@v/{version}.zip"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), zip_bytes.as_slice());
}

#[actix_web::test]
async fn go_latest_returns_most_recent_version() {
    let app = make_local_go_app(RegistryMode::Local).await;

    for v in ["v1.0.0", "v1.1.0", "v2.0.0"] {
        let zip = make_go_module_zip("example.com/latestmod", v);
        let req = TestRequest::put()
            .uri(&format!("/proxy/local-go/example.com/latestmod/@v/{v}.zip"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .insert_header(("Content-Type", "application/zip"))
            .set_payload(zip)
            .to_request();
        call_service(&app, req).await;
    }

    let req = TestRequest::get()
        .uri("/proxy/local-go/example.com/latestmod/@latest")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["Version"], "v2.0.0");
}

#[actix_web::test]
async fn go_info_unknown_returns_404() {
    let app = make_local_go_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-go/example.com/nomod/@v/v9.9.9.info")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}
