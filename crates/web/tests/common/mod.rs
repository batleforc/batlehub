//! Shared test infrastructure for the split integration test suite.
//! See the module-level docs on any sibling test file for context.
//!
//! `mod common;` is compiled independently into every sibling test binary, and
//! each binary only exercises a subset of these helpers — hence `dead_code` is
//! allowed wholesale here rather than per-item.
#![allow(dead_code, unused_imports)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::test::init_service;
use actix_web::App;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use futures::stream;
use utoipa_actix_web::AppExt;

use batlehub_adapters::auth::StaticTokenAuthProvider;
use batlehub_adapters::cache::InMemoryCacheStore;
pub use batlehub_adapters::in_memory::InMemoryTeamNamespaceStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta, NullUserTokenRepository as NullTokenRepository,
};
use batlehub_adapters::local_registry::InMemoryLocalRegistry;
use batlehub_adapters::notification::InMemoryNotificationStore;
use batlehub_config::schema::{NotificationsConfig, RegistryMode};
use batlehub_core::entities::{NamespacePackage, TeamNamespace, Visibility};
use batlehub_core::ports::BannerPort;
use batlehub_core::ports::NotificationPort;
use batlehub_core::ports::{IpBlockStore, TeamNamespacePort};
use batlehub_core::{
    entities::{PackageId, PackageMetadata, Role},
    error::CoreError,
    ports::{
        AuthProvider, CacheStore, FetchedArtifact, LocalRegistryBackend, PackageRepository,
        RegistryClient, StorageBackend, UserTokenRepository,
    },
    rules::{BlockListRule, RbacRule},
    services::{
        new_hot_lock, AdminService, HotConfig, LocalRegistryService, ProxyMetrics, ProxyService,
        RegistryPolicy, SbomService,
    },
};
use batlehub_web::services::NotificationService;
use batlehub_web::{
    configure_app, new_access_lock, AuthMiddlewareFactory, RegistryModeMap, RepoSignerMap,
};

pub struct FixedRegistry {
    registry_type: String,
}

impl FixedRegistry {
    pub fn new(registry_type: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            registry_type: registry_type.into(),
        })
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
pub const ADMIN_TOKEN: &str = "admin-token";
pub const USER_TOKEN: &str = "user-token";
pub const TEAM_A_TOKEN: &str = "team-a-token";
pub const TEAM_B_TOKEN: &str = "team-b-token";
pub const TEAM_AB_TOKEN: &str = "team-ab-token";

pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}
pub fn access_config(anonymous: &[&str], user_admin: &[&str]) -> batlehub_web::AccessConfigLock {
    let to_set = |names: &[&str]| -> std::collections::HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    };
    new_access_lock(batlehub_web::AccessConfig {
        anonymous: to_set(anonymous),
        user: to_set(user_admin),
        admin: to_set(user_admin),
        groups: std::collections::HashMap::new(),
        explore_anonymous: std::collections::HashSet::new(),
        explore_user: std::collections::HashSet::new(),
        explore_admin: std::collections::HashSet::new(),
    })
}

/// `AccessConfig` granting anonymous/user/admin access to exactly `names`,
/// with empty groups and explore overrides.
pub fn access_config_for(names: &[&str]) -> batlehub_web::AccessConfigLock {
    access_config(names, names)
}

/// Like [`access_config_for`], but also grants explore access to `names` for every role.
pub fn access_config_with_explore(names: &[&str]) -> batlehub_web::AccessConfigLock {
    let set: std::collections::HashSet<String> = names.iter().map(|s| s.to_string()).collect();
    new_access_lock(batlehub_web::AccessConfig {
        anonymous: set.clone(),
        user: set.clone(),
        admin: set.clone(),
        groups: std::collections::HashMap::new(),
        explore_anonymous: set.clone(),
        explore_user: set.clone(),
        explore_admin: set,
    })
}
pub fn registry_map_for(pairs: &[(&str, &str)]) -> batlehub_web::RegistryMap {
    batlehub_web::RegistryMap::from(
        pairs
            .iter()
            .map(|(n, t)| (n.to_string(), t.to_string()))
            .collect::<std::collections::HashMap<String, String>>(),
    )
}
pub fn test_auth_providers() -> Vec<Arc<dyn AuthProvider>> {
    vec![Arc::new(StaticTokenAuthProvider::new([
        (
            ADMIN_TOKEN.to_owned(),
            Some("admin".to_owned()),
            Role::Admin,
        ),
        (USER_TOKEN.to_owned(), Some("user-1".to_owned()), Role::User),
    ]))]
}
pub fn make_local_svc(storage: Arc<dyn StorageBackend>) -> Arc<LocalRegistryService> {
    Arc::new(LocalRegistryService {
        backend: Arc::new(InMemoryLocalRegistry::new()),
        storage,
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            ..Default::default()
        }),
        quota: None,
        ownership: None,
        team_namespace: None,
        sbom: None,
        explore_cache: None,
        access_log: None,
    })
}
pub fn rbac_policy(repo: Arc<dyn PackageRepository>) -> RegistryPolicy {
    let perms = HashMap::from([
        (Role::Anonymous, vec!["releases:read".to_owned()]),
        (
            Role::User,
            vec!["releases:read".to_owned(), "source:read".to_owned()],
        ),
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
pub struct ConfigureAppDefaults {
    pub upstream_map: batlehub_web::UpstreamMap,
    pub proxy_metrics: Arc<ProxyMetrics>,
    pub sbom_svc: Option<Arc<SbomService>>,
    pub notification_svc: Option<Arc<NotificationService>>,
    pub notification_store: Arc<dyn NotificationPort + 'static>,
    pub notifications_config: Option<NotificationsConfig>,
}

impl Default for ConfigureAppDefaults {
    fn default() -> Self {
        Self {
            upstream_map: batlehub_web::UpstreamMap::default(),
            proxy_metrics: Arc::new(ProxyMetrics::new(&[])),
            sbom_svc: None,
            notification_svc: None,
            notification_store: Arc::new(InMemoryNotificationStore::new()),
            notifications_config: None,
        }
    }
}
pub fn configure_test_app(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    token_repo: Arc<dyn UserTokenRepository>,
    access_config: batlehub_web::AccessConfigLock,
    registry_map: batlehub_web::RegistryMap,
    defaults: ConfigureAppDefaults,
) -> impl Fn(&mut utoipa_actix_web::service_config::ServiceConfig) + Clone + 'static {
    configure_app(
        proxy_svc,
        admin_svc,
        token_repo,
        None,
        access_config,
        registry_map,
        defaults.upstream_map,
        vec![],
        std::collections::HashMap::new(), // warming_map
        std::collections::HashMap::new(), // eviction_map
        defaults.proxy_metrics,
        None,
        defaults.sbom_svc,
        defaults.notification_svc,
        defaults.notification_store,
        defaults.notifications_config,
        None, // storage_admin_repo
    )
}
#[allow(clippy::too_many_arguments)]
pub async fn finish_test_app(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    token_repo: Arc<dyn UserTokenRepository>,
    access_config: batlehub_web::AccessConfigLock,
    registry_map: batlehub_web::RegistryMap,
    local_svc: Arc<LocalRegistryService>,
    mode_map: RegistryModeMap,
    cargo_indexes: batlehub_web::CargoIndexMap,
    defaults: ConfigureAppDefaults,
    auth_providers: Vec<Arc<dyn AuthProvider>>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            admin_svc,
            token_repo,
            access_config,
            registry_map,
            defaults,
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map))
        .app_data(actix_web::web::Data::new(RepoSignerMap::default()))
        .app_data(actix_web::web::Data::new(batlehub_web::VulnDbMap::default()));

    init_service(app.wrap(AuthMiddlewareFactory::new(auth_providers))).await
}
#[allow(clippy::too_many_arguments)]
pub async fn finish_test_app_with_extra<E: 'static>(
    proxy_svc: Arc<ProxyService>,
    admin_svc: Arc<AdminService>,
    token_repo: Arc<dyn UserTokenRepository>,
    access_config: batlehub_web::AccessConfigLock,
    registry_map: batlehub_web::RegistryMap,
    local_svc: Arc<LocalRegistryService>,
    mode_map: RegistryModeMap,
    cargo_indexes: batlehub_web::CargoIndexMap,
    defaults: ConfigureAppDefaults,
    extra: E,
    auth_providers: Vec<Arc<dyn AuthProvider>>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            admin_svc,
            token_repo,
            access_config,
            registry_map,
            defaults,
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(mode_map))
        .app_data(actix_web::web::Data::new(RepoSignerMap::default()))
        .app_data(actix_web::web::Data::new(batlehub_web::VulnDbMap::default()))
        .app_data(actix_web::web::Data::new(extra));

    init_service(app.wrap(AuthMiddlewareFactory::new(auth_providers))).await
}
pub async fn make_app(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    make_app_ext(repo, Arc::new(ProxyMetrics::new(&[]))).await
}

/// Variant of `make_app` that accepts a caller-supplied `proxy_metrics` so
/// that tests can inspect or mutate counters and verify the stats endpoint.
pub async fn make_app_ext(
    repo: Arc<InMemoryRepo>,
    proxy_metrics: Arc<ProxyMetrics>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [
        (
            "github".to_owned(),
            FixedRegistry::new("github") as Arc<dyn RegistryClient>,
        ),
        (
            "npm".to_owned(),
            FixedRegistry::new("npm") as Arc<dyn RegistryClient>,
        ),
        (
            "cargo".to_owned(),
            FixedRegistry::new("cargo") as Arc<dyn RegistryClient>,
        ),
        (
            "openvsx".to_owned(),
            FixedRegistry::new("openvsx") as Arc<dyn RegistryClient>,
        ),
        (
            "go".to_owned(),
            FixedRegistry::new("goproxy") as Arc<dyn RegistryClient>,
        ),
        (
            "vscode".to_owned(),
            FixedRegistry::new("vscode-marketplace") as Arc<dyn RegistryClient>,
        ),
        (
            "fj".to_owned(),
            FixedRegistry::new("forgejo") as Arc<dyn RegistryClient>,
        ),
        (
            "gl".to_owned(),
            FixedRegistry::new("gitlab") as Arc<dyn RegistryClient>,
        ),
        (
            "jb".to_owned(),
            FixedRegistry::new("jetbrains") as Arc<dyn RegistryClient>,
        ),
    ]
    .into();

    let policies: HashMap<String, Arc<RegistryPolicy>> = [
        ("github".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("npm".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("cargo".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        (
            "openvsx".to_owned(),
            Arc::new(rbac_policy(repo_dyn.clone())),
        ),
        ("go".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("vscode".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("fj".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("gl".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
        ("jb".to_owned(), Arc::new(rbac_policy(repo_dyn.clone()))),
    ]
    .into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: proxy_metrics.clone(),
        sbom: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));

    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = access_config_for(&[
        "github", "npm", "cargo", "openvsx", "go", "vscode", "fj", "gl", "jb",
    ]);
    let registry_map = registry_map_for(&[
        ("github", "github"),
        ("npm", "npm"),
        ("cargo", "cargo"),
        ("openvsx", "openvsx"),
        ("go", "goproxy"),
        ("vscode", "vscode-marketplace"),
        ("fj", "forgejo"),
        ("gl", "gitlab"),
        ("jb", "jetbrains"),
    ]);
    let cargo_indexes = batlehub_web::CargoIndexMap::default();
    finish_test_app(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults {
            proxy_metrics,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await
}
pub struct LocalRegistryAppParts {
    pub proxy_svc: Arc<ProxyService>,
    pub admin_svc: Arc<AdminService>,
    pub token_repo: Arc<dyn UserTokenRepository>,
    pub access_config: batlehub_web::AccessConfigLock,
    pub registry_map: batlehub_web::RegistryMap,
    pub local_svc: Arc<LocalRegistryService>,
    pub mode_map: RegistryModeMap,
}

pub fn local_registry_app_parts(
    name: &str,
    registry_type: &str,
    mode: RegistryMode,
    sbom_svc: Option<Arc<SbomService>>,
) -> LocalRegistryAppParts {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        name.to_owned(),
        FixedRegistry::new(registry_type) as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<RegistryPolicy>> =
        [(name.to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: sbom_svc,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));

    let mode_map = RegistryModeMap::default();
    mode_map.insert(name.to_owned(), mode);

    LocalRegistryAppParts {
        proxy_svc,
        admin_svc,
        token_repo: Arc::new(NullTokenRepository),
        access_config: access_config(&[], &[name]),
        registry_map: registry_map_for(&[(name, registry_type)]),
        local_svc,
        mode_map,
    }
}

/// Finish wiring a `make_local_<type>_app` factory: configure the routes from `parts`
/// (with the given `cargo_indexes` and optional `sbom_svc`), attach `local_svc`/`mode_map`,
/// and wrap with the standard test auth providers.
pub async fn build_local_registry_app(
    parts: LocalRegistryAppParts,
    cargo_indexes: batlehub_web::CargoIndexMap,
    sbom_svc: Option<Arc<SbomService>>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let LocalRegistryAppParts {
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        mode_map,
    } = parts;

    finish_test_app(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        mode_map,
        cargo_indexes,
        ConfigureAppDefaults {
            sbom_svc,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await
}

/// Common building blocks for a fully-wired test app with no configured registries.
pub struct EmptyAppParts {
    pub proxy_svc: Arc<ProxyService>,
    pub admin_svc: Arc<AdminService>,
    pub token_repo: Arc<dyn UserTokenRepository>,
    pub access_config: batlehub_web::AccessConfigLock,
    pub registry_map: batlehub_web::RegistryMap,
    pub cargo_indexes: batlehub_web::CargoIndexMap,
    pub local_svc: Arc<LocalRegistryService>,
}

pub fn empty_app_parts() -> EmptyAppParts {
    let repo = InMemoryRepo::new();
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig::default()),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    EmptyAppParts {
        proxy_svc,
        admin_svc: Arc::new(AdminService::new(repo_dyn)),
        token_repo: Arc::new(NullTokenRepository),
        access_config: access_config_for(&[]),
        registry_map: registry_map_for(&[]),
        cargo_indexes: batlehub_web::CargoIndexMap::default(),
        local_svc,
    }
}

pub async fn make_app_with_ip_store(
    ip_store: Arc<dyn IpBlockStore>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let EmptyAppParts {
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        cargo_indexes,
        local_svc,
    } = empty_app_parts();

    finish_test_app_with_extra(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults::default(),
        ip_store,
        test_auth_providers(),
    )
    .await
}

pub async fn make_app_with_notifications(
    notification_svc: Option<Arc<NotificationService>>,
    notification_store: Arc<dyn NotificationPort>,
    notifications_config: Option<NotificationsConfig>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let EmptyAppParts {
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        cargo_indexes,
        local_svc,
    } = empty_app_parts();

    finish_test_app(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults {
            notification_svc,
            notification_store,
            notifications_config,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await
}
pub fn make_publish_payload(name: &str, version: &str) -> Vec<u8> {
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
pub async fn make_local_registry_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    make_local_registry_app_with_sbom(mode, None).await
}

pub async fn make_local_registry_app_with_sbom(
    mode: RegistryMode,
    sbom_svc: Option<Arc<SbomService>>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let parts = local_registry_app_parts("local-cargo", "cargo", mode.clone(), sbom_svc.clone());
    // Hybrid mode requires an upstream index for config.json to succeed.
    // A dummy URL is sufficient — upstream fetches only happen on actual index lookups.
    let mut cargo_map: std::collections::HashMap<String, batlehub_web::CargoIndexProxy> =
        std::collections::HashMap::new();
    if matches!(mode, RegistryMode::Hybrid) {
        cargo_map.insert(
            "local-cargo".to_owned(),
            batlehub_web::CargoIndexProxy {
                http: reqwest::Client::new(),
                index_url: "https://index.crates.io".to_owned(),
            },
        );
    }
    let cargo_indexes = batlehub_web::CargoIndexMap::new(cargo_map);

    build_local_registry_app(parts, cargo_indexes, sbom_svc).await
}
pub async fn make_local_composer_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-composer", "composer", mode, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

/// A token for a user who is a member of group "team-alpha".
pub const NS_MEMBER_TOKEN: &str = "ns-member-token";
/// A regular user with no group membership.
pub const NS_PLAIN_USER_TOKEN: &str = "ns-plain-user-token";

pub fn team_ns_auth_providers() -> Vec<Arc<dyn AuthProvider>> {
    vec![Arc::new(
        StaticTokenAuthProvider::new([
            (
                ADMIN_TOKEN.to_owned(),
                Some("admin".to_owned()),
                Role::Admin,
            ),
            (
                NS_PLAIN_USER_TOKEN.to_owned(),
                Some("plain-user".to_owned()),
                Role::User,
            ),
        ])
        .with_group_entries([(
            NS_MEMBER_TOKEN.to_owned(),
            Some("member-user".to_owned()),
            Role::User,
            vec!["team-alpha".to_owned()],
        )]),
    )]
}

pub async fn make_local_nuget_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let mut registries: HashMap<String, Arc<dyn RegistryClient>> = HashMap::new();
    if matches!(mode, RegistryMode::Hybrid) {
        registries.insert(
            "local-nuget".to_owned(),
            FixedRegistry::new("nuget") as Arc<dyn RegistryClient>,
        );
    }
    let policies: HashMap<String, Arc<RegistryPolicy>> = [(
        "local-nuget".to_owned(),
        Arc::new(rbac_policy(repo_dyn.clone())),
    )]
    .into();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let mode_map = RegistryModeMap::default();
    mode_map.insert("local-nuget".to_owned(), mode);

    let parts = LocalRegistryAppParts {
        proxy_svc,
        admin_svc,
        token_repo: Arc::new(NullTokenRepository),
        access_config: access_config(&[], &["local-nuget"]),
        registry_map: registry_map_for(&[("local-nuget", "nuget")]),
        local_svc,
        mode_map,
    };
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

pub fn make_composer_zip(name: &str, version: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default();
        writer.start_file("composer.json", opts).unwrap();
        let json = serde_json::json!({
            "name": name,
            "version": version,
            "description": "Test package",
            "type": "library",
        });
        writer.write_all(json.to_string().as_bytes()).unwrap();
        writer.finish().unwrap();
    }
    buf.into_inner()
}
