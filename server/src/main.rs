use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::{http, web, App, HttpServer};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace as sdktrace, Resource};
use tracing::info;
use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder, TracingLogger};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::OpenApi as _;
use utoipa_actix_web::AppExt;

use batlehub_adapters::cache::InMemoryBannerStore;
#[cfg(feature = "cache-redis")]
use batlehub_adapters::cache::RedisBannerStore;
#[cfg(feature = "cache-redis")]
use batlehub_adapters::cache::RedisCacheStore;
use batlehub_adapters::cache::{InMemoryCacheStore, PgCacheStore};
use batlehub_adapters::db::PgBannerStore;
use batlehub_adapters::notification::PgNotificationStore;
use batlehub_adapters::rate_limit::{
    InMemoryIpBlockStore, InMemoryRateLimitStore, PgIpBlockStore, PgRateLimitStore,
};
#[cfg(feature = "cache-redis")]
use batlehub_adapters::rate_limit::{RedisIpBlockStore, RedisRateLimitStore};
use batlehub_adapters::{
    auth::{
        hash_static_token, ActionsOidcAuthProvider, KubernetesAuthProvider, OidcAuthProvider,
        OidcSsoFlow, StaticTokenAuthProvider, UserTokenAuthProvider,
    },
    db::{
        PgArtifactMetaRepository, PgBetaChannelStore, PgOwnershipStore, PgPackageRepository,
        PgQuotaRepository, PgSbomRepository, PgTeamNamespaceStore,
    },
    local_registry::PostgresLocalRegistry,
    registry::{
        CargoRegistryClient, ComposerRegistryClient, CondaRegistryClient, FanoutRegistryClient,
        GithubRegistryClient, GoProxyRegistryClient, MavenRegistryClient, NpmRegistryClient,
        NugetRegistryClient, OpenVsxRegistryClient, PypiRegistryClient, RubyGemsRegistryClient,
        TerraformRegistryClient, UpstreamHttpOptions, VsCodeMarketplaceRegistryClient,
    },
    sbom::HttpSbomFetcher,
    storage::{FilesystemStorageBackend, StorageRouter},
};
use batlehub_config::{
    load,
    schema::{
        AuthConfig, OtelConfig, QuotaEnforcement as ConfigQuotaEnforcement, RegistryConfig,
        RegistryMode, RuleConfig, StorageBackendConfig, StoragesConfig, UpstreamAuthConfig,
    },
};
use batlehub_core::ports::{BannerPort, NotificationPort};
use batlehub_core::services::WarmingService;
use batlehub_core::{
    entities::Role,
    ports::{
        AuthProvider, BetaChannelPort, CacheStore, IpBlockStore, RateLimitStore, SbomRepository,
        UserTokenRepository,
    },
    rules::{BlockListRule, DenyLatestRule, RbacRule, ReleaseAgeGateRule},
    services::{
        new_hot_lock, AdminService, HotSbomConfig, LocalRegistryService, ProxyMetrics,
        ProxyService, QuotaEnforcement, QuotaService, RegistryPolicy, RegistryQuotaConfig,
        SbomService, SigningConfig as CoreSigningConfig, VersioningPolicy,
    },
};
use batlehub_web::handlers::back_office::warming::WarmingServiceMap;
use batlehub_web::services::{BannerService, ConfigReloadService, NotificationService};
use batlehub_web::{
    configure_app, healthz, new_access_lock, openapi_spec, prometheus_metrics, AccessConfig,
    ApiDoc, CargoIndexMap, CargoIndexProxy, CliBinaryPath, IpBlockMiddlewareFactory,
    RateLimitMiddlewareFactory, RateLimitService, RegistryMap, RegistryModeMap, UpstreamMap,
};
use metrics_exporter_prometheus::PrometheusBuilder;

// ── Tracing span builder ──────────────────────────────────────────────────────

/// Custom root span builder that separates upstream/client errors from backend faults:
/// - 4xx → `INFO`  "upstream/client error (not a backend fault)"
/// - 5xx → `WARN`  "backend error"
struct BatleHubSpanBuilder;

impl RootSpanBuilder for BatleHubSpanBuilder {
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> tracing::Span {
        tracing_actix_web::root_span!(level = tracing::Level::INFO, request)
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: tracing::Span,
        outcome: &Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
    ) {
        let status = match outcome {
            Ok(resp) => resp.status(),
            Err(err) => err.as_response_error().status_code(),
        };
        if status.is_client_error() {
            tracing::info!(
                http.status_code = status.as_u16(),
                "upstream/client error (not a backend fault)"
            );
        } else if status.is_server_error() {
            tracing::warn!(http.status_code = status.as_u16(), "backend error");
        }
        DefaultRootSpanBuilder::on_request_end(span, outcome);
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "batlehub",
    about = "BatleHub — smart artifact hub for package registries"
)]
struct Cli {
    #[arg(short, long)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the OpenAPI spec to stdout and exit (for frontend code generation).
    DumpSpec,
    /// Hash a plain-text token with Argon2id and print the result.
    ///
    /// Use the output as the `value` in `[[auth.tokens]]` to avoid storing
    /// credentials in plain text.  Example:
    ///
    ///   batlehub hash-token my-secret-token
    HashToken {
        /// The plain-text token to hash.
        token: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle subcommands before loading config or initialising anything heavy.
    match cli.command {
        Some(Command::DumpSpec) => {
            let spec = openapi_spec();
            println!("{}", spec.to_pretty_json().expect("serialize openapi spec"));
            return Ok(());
        }
        Some(Command::HashToken { token }) => {
            println!("{}", hash_static_token(&token));
            return Ok(());
        }
        None => {}
    }

    let config_path = cli
        .config
        .or_else(|| std::env::var("BATLEHUB_CONFIG").ok())
        .unwrap_or_else(|| "config.toml".to_string());
    let config =
        load(&config_path).with_context(|| format!("loading config from '{config_path}'"))?;

    // ── Prometheus metrics recorder ───────────────────────────────────────────
    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .context("installing Prometheus metrics recorder")?;

    // ── Tracing ───────────────────────────────────────────────────────────────
    let _tracer_provider = init_tracing(config.otel.as_ref());

    tracing::info!(config = %config_path, "batlehub starting");

    // ── Database ──────────────────────────────────────────────────────────────
    let repo = Arc::new(
        PgPackageRepository::new(&config.database.url)
            .await
            .context("connecting to database")?,
    );
    repo.run_migrations().await.context("running migrations")?;

    // ── Storage ───────────────────────────────────────────────────────────────
    let storage: Arc<dyn batlehub_core::ports::StorageBackend> = match &config.storage {
        StoragesConfig::Single(backend_cfg) => {
            // Wrap in StorageRouter so artifact_storage is always tracked in the DB,
            // enabling the health endpoint to report accurate artifact counts and sizes.
            let backend = build_single_backend(backend_cfg).await?;
            let mut backends = HashMap::new();
            backends.insert("default".to_string(), backend);
            Arc::new(StorageRouter::new(
                backends,
                "default".to_string(),
                HashMap::new(),
                repo.pool(),
            ))
        }
        StoragesConfig::Multi(multi) => {
            let mut backends = HashMap::new();
            for named in &multi.backends {
                let backend = build_single_backend(&named.config).await?;
                backends.insert(named.name.clone(), backend);
            }
            if !backends.contains_key(&multi.default) {
                anyhow::bail!(
                    "storage default '{}' does not match any backend name in [[storage.backends]]",
                    multi.default
                );
            }
            let registry_assignments: HashMap<String, String> = config
                .registries
                .iter()
                .filter_map(|r| r.storage.as_ref().map(|s| (r.name.clone(), s.clone())))
                .collect();
            Arc::new(StorageRouter::new(
                backends,
                multi.default.clone(),
                registry_assignments,
                repo.pool(),
            ))
        }
    };

    // ── Auth providers ────────────────────────────────────────────────────────
    let mut auth_providers: Vec<Arc<dyn AuthProvider>> = Vec::new();
    let mut oidc_sso_flows: Vec<OidcSsoFlow> = Vec::new();

    for auth_cfg in &config.auth {
        match auth_cfg {
            AuthConfig::Token(tok) => {
                let entries = tok.tokens.iter().map(|t| {
                    let role = parse_role(&t.role);
                    (t.value.clone(), t.user_id.clone(), role)
                });
                auth_providers.push(Arc::new(StaticTokenAuthProvider::new(entries)));
                info!("configured static token auth provider");
            }
            AuthConfig::Oidc(oidc_cfg) => {
                match OidcAuthProvider::new(oidc_cfg).await {
                    Ok(provider) => {
                        if let Some(flow) = provider.sso_flow().cloned() {
                            oidc_sso_flows.push(flow);
                        }
                        auth_providers.push(Arc::new(provider));
                        tracing::info!(issuer = %oidc_cfg.issuer_url, "OIDC auth provider ready");
                    }
                    Err(e) => {
                        // Non-fatal: server starts without this OIDC provider.
                        // The /auth/oidc/login endpoint will return 503 for this
                        // provider until the server is restarted with a reachable issuer.
                        tracing::warn!(
                            issuer = %oidc_cfg.issuer_url,
                            error = %e,
                            "OIDC provider unreachable at startup — continuing without it"
                        );
                    }
                }
            }
            AuthConfig::Kubernetes(k8s_cfg) => {
                let provider = KubernetesAuthProvider::new(k8s_cfg)
                    .await
                    .context("initialising Kubernetes auth provider")?;
                auth_providers.push(Arc::new(provider));
                info!(
                    "configured Kubernetes auth provider for service account '{}'",
                    k8s_cfg.audiences.join(", ")
                );
            }
            AuthConfig::ActionsOidc(cfg) => match ActionsOidcAuthProvider::new(cfg).await {
                Ok(provider) => {
                    auth_providers.push(Arc::new(provider));
                    tracing::info!(
                        name = %cfg.name,
                        issuer = %cfg.issuer_url,
                        rules = cfg.rules.len(),
                        "Actions OIDC auth provider ready"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        issuer = %cfg.issuer_url,
                        error = %e,
                        "Actions OIDC provider unreachable at startup — continuing without it"
                    );
                }
            },
        }
    }

    // Add user-token provider (after OIDC so JWTs are validated first)
    let token_repo = repo.clone() as Arc<dyn UserTokenRepository>;
    auth_providers.push(Arc::new(UserTokenAuthProvider::new(token_repo.clone())));
    info!("configured user-token auth provider");

    // ── Cache ─────────────────────────────────────────────────────────────────
    let cache: Arc<dyn CacheStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("metadata cache: postgres");
            Arc::new(PgCacheStore::new(repo.pool()))
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config
                    .cache
                    .url
                    .as_deref()
                    .unwrap_or("redis://127.0.0.1:6379");
                tracing::info!(url, "metadata cache: redis");
                Arc::new(
                    RedisCacheStore::new(url)
                        .await
                        .context("connecting to Redis cache")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!(
                    "compiled without cache-redis feature; falling back to in-memory cache"
                );
                Arc::new(InMemoryCacheStore::new())
            }
        }
        other => {
            if other != "memory" {
                tracing::warn!(cache_type = %other, "unknown cache type, falling back to memory");
            } else {
                tracing::info!("metadata cache: memory");
            }
            Arc::new(InMemoryCacheStore::new())
        }
    };

    // ── Cargo sparse indexes ──────────────────────────────────────────────────
    // Wrapped in CargoIndexMap (Arc<RwLock<...>>) so hot-reload can swap proxy
    // settings without restarting workers.
    let cargo_index_map = {
        let mut map: HashMap<String, CargoIndexProxy> = HashMap::new();
        for reg in &config.registries {
            if reg.registry_type == "cargo" && !matches!(reg.mode, RegistryMode::Local) {
                let index = build_cargo_index(reg, config.proxy.as_ref())
                    .with_context(|| format!("building cargo index client for '{}'", reg.name))?;
                map.insert(reg.name.clone(), index);
            }
        }
        CargoIndexMap::new(map)
    };

    // ── Rate limiting ──────────────────────────────────────────────────────────
    let rate_limit_configs: std::collections::HashMap<
        String,
        batlehub_config::schema::RateLimitConfig,
    > = config
        .registries
        .iter()
        .filter_map(|r| r.rate_limit.clone().map(|rl| (r.name.clone(), rl)))
        .collect();
    let rate_limit_store: Arc<dyn RateLimitStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("rate limit store: postgres");
            Arc::new(PgRateLimitStore::new(repo.pool()))
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config
                    .cache
                    .url
                    .as_deref()
                    .unwrap_or("redis://127.0.0.1:6379");
                tracing::info!(url, "rate limit store: redis");
                Arc::new(
                    RedisRateLimitStore::new(url)
                        .await
                        .context("connecting to Redis rate limit store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!("compiled without cache-redis feature; falling back to in-memory rate limit store");
                Arc::new(InMemoryRateLimitStore::new())
            }
        }
        other => {
            if other != "memory" {
                tracing::warn!(cache_type = %other, "unknown cache type for rate limit store, falling back to memory");
            }
            Arc::new(InMemoryRateLimitStore::new())
        }
    };
    let rate_limit_svc = Arc::new(RateLimitService::new(&rate_limit_configs, rate_limit_store));

    // ── Services ──────────────────────────────────────────────────────────────
    let registry_names: Vec<String> = config.registries.iter().map(|r| r.name.clone()).collect();
    let proxy_metrics = Arc::new(ProxyMetrics::new(&registry_names));

    let artifact_meta = Arc::new(PgArtifactMetaRepository::new(repo.pool()));

    let admin_svc = Arc::new(AdminService::new(
        repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>
    ));

    let local_registry_backend = Arc::new(PostgresLocalRegistry::new(repo.pool()));
    let quota_svc = Arc::new(build_quota_service(repo.pool(), &config.registries));
    let ownership_store = Arc::new(PgOwnershipStore::new(repo.pool()))
        as Arc<dyn batlehub_core::ports::OwnershipPort>;
    let beta_channel_store: Arc<dyn BetaChannelPort> =
        Arc::new(PgBetaChannelStore::new(repo.pool()));
    let team_namespace_store: Arc<dyn batlehub_core::ports::TeamNamespacePort> =
        Arc::new(PgTeamNamespaceStore::new(repo.pool()));

    // ── Shared hot-reloadable config ──────────────────────────────────────────
    let (init_hot, init_access, registry_map, registry_mode_map, upstream_map) = build_hot_bundle(
        &config,
        &beta_channel_store,
        &(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>),
    )?;

    // Clone warming clients before the hot lock consumes the HashMap.
    let warming_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> = init_hot
        .registries
        .iter()
        .map(|(k, v)| (k.clone(), Arc::clone(v)))
        .collect();

    let hot = new_hot_lock(init_hot);

    // ── SBOM service ─────────────────────────────────────────────────────────
    let sbom_repo: Arc<dyn SbomRepository> = Arc::new(PgSbomRepository::new(repo.pool()));
    #[cfg(feature = "sbom")]
    let sbom_extractor: Option<Arc<dyn batlehub_core::ports::SbomExtractor>> =
        Some(Arc::new(batlehub_adapters::sbom::ArchiveSbomExtractor));
    #[cfg(not(feature = "sbom"))]
    let sbom_extractor: Option<Arc<dyn batlehub_core::ports::SbomExtractor>> = None;
    let sbom_http = reqwest::Client::builder()
        .user_agent("batlehub/sbom")
        .build()
        .context("building SBOM HTTP client")?;
    let sbom_svc = Arc::new(SbomService::new(
        sbom_repo,
        sbom_extractor,
        Some(Arc::new(HttpSbomFetcher::new(sbom_http))),
    ));

    let proxy_svc = Arc::new(ProxyService {
        hot: Arc::clone(&hot),
        storage: storage.clone(),
        cache: cache.clone(),
        repo: repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>,
        artifact_meta,
        metrics: Arc::clone(&proxy_metrics),
        sbom: Some(Arc::clone(&sbom_svc)),
    });

    // ── IP blocking store ─────────────────────────────────────────────────────
    let ip_block_store: Arc<dyn IpBlockStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("ip block store: postgres");
            Arc::new(PgIpBlockStore::new(repo.pool()))
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config
                    .cache
                    .url
                    .as_deref()
                    .unwrap_or("redis://127.0.0.1:6379");
                tracing::info!(url, "ip block store: redis");
                Arc::new(
                    RedisIpBlockStore::new(url)
                        .await
                        .context("connecting to Redis ip block store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!("compiled without cache-redis feature; falling back to in-memory ip block store");
                Arc::new(InMemoryIpBlockStore::new())
            }
        }
        _ => Arc::new(InMemoryIpBlockStore::new()),
    };
    let ip_blocking_cfg = config.ip_blocking.clone();
    let local_svc = Arc::new(LocalRegistryService {
        backend: local_registry_backend,
        storage: storage.clone(),
        hot: Arc::clone(&hot),
        quota: Some(Arc::clone(&quota_svc)),
        ownership: Some(ownership_store),
        team_namespace: Some(Arc::clone(&team_namespace_store)),
        sbom: Some(Arc::clone(&sbom_svc)),
        explore_cache: Some(Arc::clone(&admin_svc.explore_cache)),
    });

    // ── Warming services ──────────────────────────────────────────────────────
    let mut warming_map: WarmingServiceMap = HashMap::new();
    for reg in &config.registries {
        if let Some(client) = warming_clients.get(&reg.name) {
            let warming_svc = Arc::new(WarmingService {
                client: Arc::clone(client),
                storage: storage.clone(),
                artifact_meta: Arc::new(PgArtifactMetaRepository::new(repo.pool()))
                    as Arc<dyn batlehub_core::ports::ArtifactMetaRepository>,
                registry_name: reg.name.clone(),
                latest_n: reg.cache.warm_latest_n,
                concurrency: reg.cache.warm_concurrency,
            });
            warming_map.insert(reg.name.clone(), warming_svc);
        }
    }

    // ── Access config ─────────────────────────────────────────────────────────
    let access_config = new_access_lock(init_access);

    // ── Hot reload & banner ───────────────────────────────────────────────────
    let hot_reload_enabled = std::env::var("BATLEHUB_DISABLE_HOT_RELOAD")
        .map(|v| v != "1" && v.to_lowercase() != "true")
        .unwrap_or(true);

    let banner_store: Arc<dyn BannerPort> = match config.cache.cache_type.as_str() {
        "postgres" => Arc::new(PgBannerStore::new(repo.pool())),
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config
                    .cache
                    .url
                    .as_deref()
                    .unwrap_or("redis://127.0.0.1:6379");
                Arc::new(
                    RedisBannerStore::new(url)
                        .await
                        .context("connecting to Redis banner store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            Arc::new(InMemoryBannerStore::new())
        }
        _ => Arc::new(InMemoryBannerStore::new()),
    };
    let banner_svc = Arc::new(BannerService::new(banner_store));

    // ── Notifications ─────────────────────────────────────────────────────────
    let notification_store: Arc<dyn NotificationPort> =
        Arc::new(PgNotificationStore::new(repo.pool()));

    let notifications_config = config.notifications.clone();
    let notification_svc: Option<Arc<NotificationService>> = notifications_config
        .as_ref()
        .filter(|nc| nc.enabled)
        .map(|nc| {
            Arc::new(NotificationService::new(
                Arc::clone(&notification_store),
                nc,
            ))
        });

    // Builder closure capturing the deps needed to rebuild HotConfig + AccessConfig + CargoIndexMap.
    let hot_builder: batlehub_web::services::HotConfigBuilder = {
        let beta_channel_store = Arc::clone(&beta_channel_store);
        let repo_for_builder = repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>;
        Arc::new(move |cfg: &batlehub_config::schema::AppConfig| {
            let (hot, access, rm, rmm, um) =
                build_hot_bundle(cfg, &beta_channel_store, &repo_for_builder)?;
            let mut cargo_map: HashMap<String, CargoIndexProxy> = HashMap::new();
            for reg in &cfg.registries {
                if reg.registry_type == "cargo" && !matches!(reg.mode, RegistryMode::Local) {
                    let index = build_cargo_index(reg, cfg.proxy.as_ref())
                        .with_context(|| {
                            format!("building cargo index client for '{}'", reg.name)
                        })?;
                    cargo_map.insert(reg.name.clone(), index);
                }
            }
            Ok((hot, access, rm, rmm, um, CargoIndexMap::new(cargo_map)))
        })
    };

    let reload_svc = Arc::new(ConfigReloadService::new(
        Arc::clone(&hot),
        Arc::clone(&access_config),
        registry_map.clone(),
        registry_mode_map.clone(),
        upstream_map.clone(),
        cargo_index_map.clone(),
        config_path.clone(),
        Some(repo.pool()),
        hot_reload_enabled,
        hot_builder,
        Some(Arc::clone(&banner_svc)),
    ));

    if hot_reload_enabled {
        spawn_config_watcher(config_path.clone(), Arc::clone(&reload_svc));
        tracing::info!("hot reload: enabled (watching {})", config_path);
    } else {
        tracing::info!("hot reload: disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
    }

    // ── HTTP server ───────────────────────────────────────────────────────────
    let bind_addr = format!("{}:{}", config.server.host, config.server.port);
    let static_dir = config.server.static_dir.clone();
    let cli_binary_path = config
        .server
        .cli_binary_path
        .as_deref()
        .map(std::path::PathBuf::from);
    let cors_allowed_origins = config
        .server
        .cors_allowed_origins
        .clone()
        .unwrap_or_default();
    let db_pool = repo.pool();

    tracing::info!(addr = %bind_addr, "listening");

    // Spawn startup warming tasks (non-blocking; server starts immediately).
    for reg in &config.registries {
        if !reg.cache.warm_packages.is_empty() {
            if let Some(svc) = warming_map.get(&reg.name) {
                let svc = Arc::clone(svc);
                let packages = reg.cache.warm_packages.clone();
                let name = reg.name.clone();
                tokio::spawn(async move {
                    tracing::info!(registry = %name, "warming: startup warming started");
                    let report = svc.warm_all(&packages).await;
                    tracing::info!(
                        registry = %name,
                        warmed = report.warmed,
                        skipped = report.skipped,
                        errors = report.errors,
                        "warming: startup warming complete"
                    );
                });
            }
        }
    }

    let reload_svc_for_server = Arc::clone(&reload_svc);
    let banner_svc_for_server = Arc::clone(&banner_svc);
    let notification_svc_for_server = notification_svc.clone();
    let notification_store_for_server = Arc::clone(&notification_store);
    let notifications_config_for_server = notifications_config.clone();
    let notification_svc_for_shutdown = notification_svc.clone();
    HttpServer::new(move || {
        let configure = configure_app(
            proxy_svc.clone(),
            admin_svc.clone(),
            token_repo.clone(),
            Some(db_pool.clone()),
            access_config.clone(),
            registry_map.clone(),
            upstream_map.clone(),
            oidc_sso_flows.clone(),
            warming_map.clone(),
            Arc::clone(&proxy_metrics),
            Some(prometheus_handle.clone()),
            Some(sbom_svc.clone()),
            notification_svc_for_server.clone(),
            Arc::clone(&notification_store_for_server),
            notifications_config_for_server.clone(),
        );
        let static_dir_inner = static_dir.clone();
        let cli_binary_path_inner = cli_binary_path.clone();
        let cargo_indexes_inner = cargo_index_map.clone();
        let local_svc_inner = local_svc.clone();
        let quota_svc_inner = Arc::clone(&quota_svc);
        let registry_mode_map_inner = registry_mode_map.clone();
        let ip_block_store_inner = Arc::clone(&ip_block_store);
        let beta_channel_store_inner = Arc::clone(&beta_channel_store);
        let team_namespace_store_inner = Arc::clone(&team_namespace_store);
        let ip_blocking_cfg_inner = ip_blocking_cfg.clone();
        let reload_svc_inner = Arc::clone(&reload_svc_for_server);
        let banner_svc_inner = Arc::clone(&banner_svc_for_server);

        let (app, openapi) = App::new()
            .into_utoipa_app()
            .openapi(ApiDoc::openapi())
            .configure(configure)
            .split_for_parts();

        // Register app-data and non-OpenAPI routes that are handled outside configure_app.
        let mut app = app
            .app_data(web::Data::new(cargo_indexes_inner))  // web::Data<CargoIndexMap>
            .app_data(web::Data::new(local_svc_inner))
            .app_data(web::Data::new(quota_svc_inner))
            .app_data(web::Data::new(registry_mode_map_inner))
            .app_data(web::Data::new(ip_block_store_inner))
            .app_data(web::Data::new(beta_channel_store_inner))
            .app_data(web::Data::new(team_namespace_store_inner))
            .app_data(web::Data::new(reload_svc_inner))
            .app_data(web::Data::new(banner_svc_inner))
            .service(prometheus_metrics)
            .service(healthz);

        if let Some(path) = cli_binary_path_inner {
            app = app.app_data(web::Data::new(CliBinaryPath(path)));
        }

        let cors_base = Cors::default()
            .allowed_methods(vec!["GET", "POST", "PUT", "HEAD", "OPTIONS", "DELETE"])
            .allowed_headers(vec![
                http::header::AUTHORIZATION,
                http::header::CONTENT_TYPE,
                http::header::ACCEPT,
            ])
            .max_age(3600);
        let cors = if cors_allowed_origins.is_empty() {
            cors_base.allow_any_origin()
        } else {
            cors_allowed_origins
                .iter()
                .fold(cors_base, |c, origin| c.allowed_origin(origin))
        };

        let enabled = ip_blocking_cfg_inner.as_ref().is_some_and(|c| c.enabled);
        let ip_block_cfg_for_mw = ip_blocking_cfg_inner.clone().unwrap_or_default();

        // Middleware runs in reverse registration order (last-registered = outermost = first to run).
        // Correct chain (outer→inner): ip_block → cors → auth (sets Identity) → rate_limit (reads Identity) → tracing → handler
        app.wrap(TracingLogger::<BatleHubSpanBuilder>::new())
            .wrap(RateLimitMiddlewareFactory::new(rate_limit_svc.clone()))
            .wrap(batlehub_web::AuthMiddlewareFactory::new(
                auth_providers.clone(),
            ))
            .wrap(cors)
            // IP blocking is the outermost middleware — it runs before auth so blocked IPs
            // never reach the auth stack.
            .wrap(actix_web::middleware::Condition::new(
                enabled,
                IpBlockMiddlewareFactory::new(Arc::clone(&ip_block_store), ip_block_cfg_for_mw),
            ))
            .service(batlehub_web::scalar(openapi))
            .configure(move |cfg| {
                if let Some(ref dir) = static_dir_inner {
                    cfg.service(
                        actix_files::Files::new("/", dir)
                            .index_file("index.html")
                            .use_last_modified(true),
                    );
                }
            })
    })
    .bind(&bind_addr)
    .with_context(|| format!("binding to {bind_addr}"))?
    .run()
    .await
    .context("HTTP server error")?;

    // Drain any in-flight notification tasks before the runtime exits.
    if let Some(svc) = &notification_svc_for_shutdown {
        svc.shutdown().await;
    }

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn build_single_backend(
    cfg: &StorageBackendConfig,
) -> Result<Arc<dyn batlehub_core::ports::StorageBackend>> {
    match cfg {
        StorageBackendConfig::Filesystem(fs) => {
            let backend = FilesystemStorageBackend::new(&fs.path)
                .await
                .with_context(|| format!("initialising filesystem storage at '{}'", fs.path))?;
            Ok(Arc::new(backend))
        }
        StorageBackendConfig::S3(_s3) => {
            #[cfg(feature = "storage-s3")]
            {
                use batlehub_adapters::storage::S3StorageBackend;
                let backend = S3StorageBackend::new(_s3).await.with_context(|| {
                    format!("initialising S3 storage for bucket '{}'", _s3.bucket)
                })?;
                return Ok(Arc::new(backend));
            }
            #[cfg(not(feature = "storage-s3"))]
            anyhow::bail!("S3 storage requires the 'storage-s3' feature flag at compile time");
        }
    }
}

fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

fn upstream_options(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> UpstreamHttpOptions {
    let (bearer_token, basic_auth, custom_header) = match &reg.upstream_auth {
        Some(UpstreamAuthConfig::Bearer(b)) => (Some(b.token.clone()), None, None),
        Some(UpstreamAuthConfig::Basic(b)) => {
            (None, Some((b.username.clone(), b.password.clone())), None)
        }
        Some(UpstreamAuthConfig::Header(h)) => {
            (None, None, Some((h.name.clone(), h.value.clone())))
        }
        None => (None, None, None),
    };
    // Per-registry proxy takes precedence; fall back to the global proxy.
    let proxy = reg.proxy.as_ref().or(global_proxy);
    UpstreamHttpOptions {
        bearer_token,
        basic_auth,
        custom_header,
        ca_cert_path: reg.tls.as_ref().and_then(|t| t.ca_cert_path.clone()),
        search_url: reg.search_url.clone(),
        proxy_url: proxy.map(|p| p.url.clone()),
        proxy_username: proxy.and_then(|p| p.username.clone()),
        proxy_password: proxy.and_then(|p| p.password.clone()),
        no_proxy: proxy.and_then(|p| p.no_proxy.clone()),
    }
}

fn build_cargo_index(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> anyhow::Result<CargoIndexProxy> {
    let index_url = if let Some(ref url) = reg.index_url {
        url.clone()
    } else {
        let upstream = reg
            .upstreams
            .first()
            .map(|s| s.as_str())
            .unwrap_or("https://crates.io");
        if upstream.contains("crates.io") {
            "https://index.crates.io".to_owned()
        } else {
            upstream.to_owned()
        }
    };
    let opts = upstream_options(reg, global_proxy);
    let http = batlehub_adapters::registry::apply_upstream_options(
        reqwest::Client::builder().user_agent("batlehub/0.1"),
        &opts,
    )?;
    tracing::info!(index_url = %index_url, "cargo sparse index proxy configured");
    Ok(CargoIndexProxy { http, index_url })
}

fn build_registry_client(
    reg: &RegistryConfig,
    global_proxy: Option<&batlehub_config::schema::UpstreamProxyConfig>,
) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
    fn resolve_urls(configured: &[String], default: &str) -> Vec<String> {
        if configured.is_empty() {
            vec![default.to_owned()]
        } else {
            configured.to_vec()
        }
    }

    fn make_one(
        registry_type: &str,
        url: &str,
        opts: &UpstreamHttpOptions,
    ) -> anyhow::Result<Arc<dyn batlehub_core::ports::RegistryClient>> {
        let client: Arc<dyn batlehub_core::ports::RegistryClient> = match registry_type {
            "github" => Arc::new(GithubRegistryClient::new(url, opts)?),
            "npm" => Arc::new(NpmRegistryClient::new(url, opts)?),
            "cargo" => Arc::new(CargoRegistryClient::new(url, opts)?),
            "nuget" => Arc::new(NugetRegistryClient::new(url, opts)?),
            "openvsx" => Arc::new(OpenVsxRegistryClient::new(url, opts)?),
            "goproxy" => Arc::new(GoProxyRegistryClient::new(url, opts)?),
            "vscode-marketplace" => Arc::new(VsCodeMarketplaceRegistryClient::new(url, opts)?),
            "maven" => Arc::new(MavenRegistryClient::new(url, opts)?),
            "terraform" => Arc::new(TerraformRegistryClient::new(url, opts)?),
            "rubygems" => Arc::new(RubyGemsRegistryClient::new(url, opts)?),
            "composer" => Arc::new(ComposerRegistryClient::new(url, opts)?),
            "pypi" => Arc::new(PypiRegistryClient::new(url, opts)?),
            "conda" => Arc::new(CondaRegistryClient::new(url, opts)?),
            other => {
                anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in")
            }
        };
        Ok(client)
    }

    let opts = upstream_options(reg, global_proxy);

    let urls = match reg.registry_type.as_str() {
        "github" => resolve_urls(&reg.upstreams, "https://api.github.com"),
        "npm" => resolve_urls(&reg.upstreams, "https://registry.npmjs.org"),
        "cargo" => resolve_urls(&reg.upstreams, "https://crates.io"),
        "nuget" => resolve_urls(&reg.upstreams, "https://api.nuget.org"),
        "openvsx" => resolve_urls(&reg.upstreams, "https://open-vsx.org"),
        "goproxy" => resolve_urls(&reg.upstreams, "https://proxy.golang.org"),
        "vscode-marketplace" => {
            resolve_urls(&reg.upstreams, "https://marketplace.visualstudio.com")
        }
        "maven" => resolve_urls(&reg.upstreams, "https://repo1.maven.org/maven2"),
        "terraform" => resolve_urls(&reg.upstreams, "https://registry.terraform.io"),
        "rubygems" => resolve_urls(&reg.upstreams, "https://rubygems.org"),
        "composer" => resolve_urls(&reg.upstreams, "https://repo.packagist.org"),
        "pypi" => resolve_urls(&reg.upstreams, "https://pypi.org"),
        "conda" => resolve_urls(&reg.upstreams, "https://conda.anaconda.org"),
        other => {
            anyhow::bail!("registry type '{other}' is configured but no adapter is compiled in")
        }
    };

    if urls.len() == 1 {
        make_one(&reg.registry_type, &urls[0], &opts)
    } else {
        let clients = urls
            .iter()
            .map(|u| make_one(&reg.registry_type, u, &opts))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Arc::new(FanoutRegistryClient::new(
            &reg.registry_type,
            clients,
        )))
    }
}

fn build_policy(
    reg: &RegistryConfig,
    repo: Arc<dyn batlehub_core::ports::PackageRepository>,
) -> RegistryPolicy {
    let mut rules: Vec<Box<dyn batlehub_core::rules::Rule>> = Vec::new();

    // 1. RBAC rule (always first)
    let rbac_perms = HashMap::from([
        (Role::Anonymous, reg.rbac.anonymous.clone()),
        (Role::User, reg.rbac.user.clone()),
        (Role::Admin, reg.rbac.admin.clone()),
    ]);
    rules.push(Box::new(
        RbacRule::new(rbac_perms).with_groups(reg.rbac.groups.clone()),
    ));

    // 2. Block list rule (always second)
    rules.push(Box::new(BlockListRule::new(repo)));

    // 3. Optional registry-specific rules from config
    for rule_cfg in &reg.rules {
        match rule_cfg {
            RuleConfig::ReleaseAgeGate(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(
                    ReleaseAgeGateRule::new(Duration::from_secs(cfg.min_age_secs), bypass)
                        .with_deny_missing_timestamp(cfg.deny_missing_timestamp),
                ));
            }
            RuleConfig::RequireSignedRelease(cfg) => {
                if cfg.enabled {
                    tracing::warn!(
                        "require_signed_release rule is configured but not yet implemented"
                    );
                }
            }
            RuleConfig::DenyLatest(cfg) => {
                let bypass: Vec<Role> = cfg.bypass_roles.iter().map(|r| parse_role(r)).collect();
                rules.push(Box::new(DenyLatestRule::new(bypass)));
            }
        }
    }

    RegistryPolicy {
        metadata_ttl: Some(Duration::from_secs(reg.cache.metadata_ttl_secs)),
        firewall_only: reg.firewall_only,
        serve_stale_metadata: reg.cache.serve_stale,
        artifact_ttl: reg.cache.artifact_ttl_secs.map(Duration::from_secs),
        rules,
    }
}

/// Build per-registry `VersioningPolicy` map from registries that have a `[versioning]` section.
fn build_versioning_map(registries: &[RegistryConfig]) -> HashMap<String, VersioningPolicy> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.versioning.as_ref().map(|v| {
                let pattern = v
                    .version_pattern
                    .as_deref()
                    .and_then(|pat| match regex::Regex::new(pat) {
                        Ok(re) => Some(re),
                        Err(e) => {
                            tracing::warn!(
                                "invalid version_pattern for registry '{}': {e}",
                                reg.name
                            );
                            None
                        }
                    });
                (
                    reg.name.clone(),
                    VersioningPolicy {
                        enforce_semver: v.enforce_semver,
                        allow_prerelease: v.allow_prerelease,
                        version_pattern: pattern,
                    },
                )
            })
        })
        .collect()
}

/// Build per-registry `SigningConfig` map from registries that have a `[signing]` section.
fn build_signing_map(registries: &[RegistryConfig]) -> HashMap<String, CoreSigningConfig> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.signing.as_ref().map(|s| {
                (
                    reg.name.clone(),
                    CoreSigningConfig {
                        required: s.required,
                        allowed_types: s.allowed_types.clone(),
                    },
                )
            })
        })
        .collect()
}

/// Build per-registry `SbomConfig` map from registries that have `[sbom]` configured.
fn build_sbom_map(registries: &[RegistryConfig]) -> HashMap<String, HotSbomConfig> {
    registries
        .iter()
        .filter_map(|reg| {
            reg.sbom.as_ref().map(|s| {
                (
                    reg.name.clone(),
                    HotSbomConfig {
                        enabled: s.enabled,
                        formats: s.formats.clone(),
                        required: s.required,
                        fetch_upstream: s.fetch_upstream,
                        registry_type: reg.registry_type.clone(),
                    },
                )
            })
        })
        .collect()
}

/// Build per-registry `BetaChannelPort` map from registries that have `beta_channel.enabled = true`.
///
/// Each enabled registry gets a clone of the same shared store Arc; no new connections are opened.
fn build_beta_channel_map(
    store: Arc<dyn BetaChannelPort>,
    registries: &[RegistryConfig],
) -> HashMap<String, Arc<dyn BetaChannelPort>> {
    registries
        .iter()
        .filter(|reg| reg.beta_channel.as_ref().is_some_and(|bc| bc.enabled))
        .map(|reg| (reg.name.clone(), Arc::clone(&store)))
        .collect()
}

/// Return the first upstream URL for registry types that require one for audit pass-through
/// (npm, terraform, pypi, conda). Returns `None` for all other types.
fn upstream_url_for(reg: &RegistryConfig) -> Option<String> {
    let default_url = match reg.registry_type.as_str() {
        "npm" => "https://registry.npmjs.org",
        "terraform" => "https://registry.terraform.io",
        "pypi" => "https://pypi.org",
        "conda" => "https://conda.anaconda.org",
        "nuget" => "https://api.nuget.org",
        _ => return None,
    };
    Some(
        reg.upstreams
            .first()
            .cloned()
            .unwrap_or_else(|| default_url.to_owned()),
    )
}

/// Build the complete hot-reloadable bundle from a config snapshot.
///
/// Called both at startup (via direct call) and on reload (via the `HotConfigBuilder` closure).
fn build_hot_bundle(
    cfg: &batlehub_config::schema::AppConfig,
    beta_channel_store: &Arc<dyn BetaChannelPort>,
    repo: &Arc<dyn batlehub_core::ports::PackageRepository>,
) -> anyhow::Result<(
    batlehub_core::services::HotConfig,
    AccessConfig,
    RegistryMap,
    RegistryModeMap,
    UpstreamMap,
)> {
    let mut reg_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> =
        HashMap::new();
    let mut reg_policies: HashMap<String, Arc<RegistryPolicy>> = HashMap::new();
    let mut reg_type_map: HashMap<String, String> = HashMap::new();
    let mut reg_mode_map: HashMap<String, RegistryMode> = HashMap::new();
    let mut upstream_map: HashMap<String, String> = HashMap::new();

    for reg in &cfg.registries {
        let client = build_registry_client(reg, cfg.proxy.as_ref())
            .with_context(|| format!("building registry client for '{}'", reg.name))?;
        reg_clients.insert(reg.name.clone(), client);
        reg_policies.insert(
            reg.name.clone(),
            Arc::new(build_policy(reg, Arc::clone(repo))),
        );
        reg_type_map.insert(reg.name.clone(), reg.registry_type.clone());
        reg_mode_map.insert(reg.name.clone(), reg.mode.clone());
        if let Some(url) = upstream_url_for(reg) {
            upstream_map.insert(reg.name.clone(), url);
        }
    }

    let hot = batlehub_core::services::HotConfig {
        registries: reg_clients,
        policies: reg_policies,
        versioning: build_versioning_map(&cfg.registries),
        signing: build_signing_map(&cfg.registries),
        sbom: build_sbom_map(&cfg.registries),
        beta_channel: build_beta_channel_map(Arc::clone(beta_channel_store), &cfg.registries),
        max_artifact_size_bytes: cfg.limits.max_artifact_size_bytes,
    };

    Ok((
        hot,
        build_access_config(cfg),
        RegistryMap::from(reg_type_map),
        RegistryModeMap::from(reg_mode_map),
        UpstreamMap::from(upstream_map),
    ))
}

/// Build the `AccessConfig` from a full app config (used at startup and on reload).
fn build_access_config(config: &batlehub_config::schema::AppConfig) -> AccessConfig {
    let mut group_access: HashMap<String, HashSet<String>> = HashMap::new();
    for r in &config.registries {
        for group_name in r.rbac.groups.keys() {
            group_access
                .entry(group_name.clone())
                .or_default()
                .insert(r.name.clone());
        }
    }
    AccessConfig {
        anonymous: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        user: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty())
            .map(|r| r.name.clone())
            .collect(),
        admin: config
            .registries
            .iter()
            .filter(|r| {
                !r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty() || !r.rbac.admin.is_empty()
            })
            .map(|r| r.name.clone())
            .collect(),
        groups: group_access,
        explore_anonymous: config
            .registries
            .iter()
            .filter(|r| !r.rbac.anonymous.is_empty() && r.rbac.explore.anonymous)
            .map(|r| r.name.clone())
            .collect(),
        explore_user: config
            .registries
            .iter()
            .filter(|r| {
                (!r.rbac.anonymous.is_empty() || !r.rbac.user.is_empty()) && r.rbac.explore.user
            })
            .map(|r| r.name.clone())
            .collect(),
        explore_admin: config
            .registries
            .iter()
            .filter(|r| {
                (!r.rbac.anonymous.is_empty()
                    || !r.rbac.user.is_empty()
                    || !r.rbac.admin.is_empty())
                    && r.rbac.explore.admin
            })
            .map(|r| r.name.clone())
            .collect(),
    }
}

/// Spawn a background task that watches the config file and loads a pending reload
/// when the file changes. The task runs until the process exits.
fn spawn_config_watcher(config_path: String, reload_svc: Arc<ConfigReloadService>) {
    use batlehub_web::services::ReloadSource;
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;
    use std::time::Duration as StdDuration;

    // Bridge: OS thread sends a unit notification; async task receives and reloads.
    // Using an unbounded tokio channel so the OS thread never blocks on send.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // ── OS thread — owns the blocking file watcher ────────────────────────────
    // Runs recv_timeout in a real OS thread, never touching the tokio scheduler.
    // Exits cleanly when event_tx.is_closed() (i.e., the async task was dropped).
    std::thread::Builder::new()
        .name("config-watcher".to_owned())
        .spawn(move || {
            let (notify_tx, notify_rx) = channel();
            let mut watcher = match RecommendedWatcher::new(
                notify_tx,
                NotifyConfig::default().with_poll_interval(StdDuration::from_secs(2)),
            ) {
                Ok(w) => w,
                Err(e) => {
                    tracing::error!(error = %e, "config file watcher init failed");
                    return;
                }
            };
            if let Err(e) = watcher.watch(
                std::path::Path::new(&config_path),
                RecursiveMode::NonRecursive,
            ) {
                tracing::error!(
                    error = %e,
                    "config file watcher: failed to watch {config_path}"
                );
                return;
            }
            tracing::info!(path = %config_path, "config file watcher started");

            loop {
                // Short timeout so we check event_tx.is_closed() frequently.
                match notify_rx.recv_timeout(StdDuration::from_secs(2)) {
                    Ok(_) => {
                        // Debounce: drain any queued events before signalling.
                        while notify_rx.try_recv().is_ok() {}
                        if event_tx.send(()).is_err() {
                            // Async side shut down — exit cleanly.
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if event_tx.is_closed() {
                            break; // Async side shut down.
                        }
                    }
                    Err(_) => break, // Watcher channel error.
                }
            }

            // watcher is dropped here, unregistering the OS file watch.
            tracing::info!("config file watcher stopped");
        })
        .expect("failed to spawn config-watcher thread");

    // ── Async task — handles reload logic ─────────────────────────────────────
    // Receives notifications from the OS thread via a non-blocking tokio channel.
    // When the tokio runtime shuts down this task is dropped, which closes
    // event_rx, causing the OS thread to detect the closed channel and exit.
    tokio::spawn(async move {
        while let Some(()) = event_rx.recv().await {
            tracing::info!("config file changed, loading pending reload");
            match reload_svc.load_pending(ReloadSource::FileWatcher).await {
                Ok(diff) => tracing::info!(
                    added = diff.added_registries.len(),
                    removed = diff.removed_registries.len(),
                    "pending reload ready — confirm at POST /api/v1/admin/config/pending/apply"
                ),
                Err(e) => tracing::warn!(error = %e, "config file reload validation failed"),
            }
        }
        // Channel closed (runtime shutting down): expire any pending reload.
        reload_svc.expire_pending_if_stale();
        tracing::debug!("config reload task exiting");
    });
}

/// Build a `QuotaService` from the registries that have a `[quota]` section.
fn build_quota_service(pool: sqlx::PgPool, registries: &[RegistryConfig]) -> QuotaService {
    let repo = Arc::new(PgQuotaRepository::new(pool));
    let configs = registries
        .iter()
        .filter_map(|reg| {
            reg.quota.as_ref().map(|q| {
                let enforcement = match q.enforcement {
                    ConfigQuotaEnforcement::Block => QuotaEnforcement::Block,
                    ConfigQuotaEnforcement::Warn => QuotaEnforcement::Warn,
                };
                (
                    reg.name.clone(),
                    RegistryQuotaConfig {
                        max_storage_bytes_per_user: q.max_storage_bytes_per_user,
                        max_packages_per_user: q.max_packages_per_user,
                        warn_threshold: q.warn_threshold_pct.clamp(1, 100) as f64 / 100.0,
                        enforcement,
                    },
                )
            })
        })
        .collect();
    QuotaService::new(repo, configs)
}

/// Initialise tracing.  Returns the `TracerProvider` when OTLP is configured
/// so the caller can keep it alive for the process lifetime and flush on exit.
fn init_tracing(otel_cfg: Option<&OtelConfig>) -> Option<sdktrace::SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let (otel_layer, provider) = match otel_cfg {
        Some(cfg) => {
            match build_otlp_provider(cfg) {
                Ok(p) => {
                    use opentelemetry::trace::TracerProvider as _;
                    let tracer = p.tracer(cfg.service_name.clone());
                    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
                    (Some(layer), Some(p))
                }
                Err(e) => {
                    // Don't crash on misconfigured OTLP; just warn and continue.
                    eprintln!("WARN: failed to build OTLP exporter: {e}");
                    (None, None)
                }
            }
        }
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer)
        .init();

    provider
}

fn build_otlp_provider(cfg: &OtelConfig) -> anyhow::Result<sdktrace::SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&cfg.endpoint)
        .build()?;

    let resource = Resource::builder_empty()
        .with_service_name(cfg.service_name.clone())
        .build();

    let provider = sdktrace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build();

    Ok(provider)
}
