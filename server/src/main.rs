mod builders;
mod hot_config;
mod server_factory;
mod setup;
mod stores;
mod watcher;

#[cfg(feature = "jemalloc")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use metrics_exporter_prometheus::PrometheusBuilder;

use batlehub_adapters::db::{
    PgArtifactMetaRepository, PgBetaChannelStore, PgOwnershipStore, PgPackageRepository,
    PgTeamNamespaceStore, PgVulnerabilityRepository,
};
use batlehub_adapters::local_registry::PostgresLocalRegistry;
use batlehub_adapters::vulnerability::OsvScanner;
use batlehub_core::ports::{BetaChannelPort, UserTokenRepository, VulnerabilityRepository};
use batlehub_core::services::{
    new_hot_lock, AdminService, LocalRegistryService, ProxyMetrics, ProxyService,
    VulnerabilityScanService,
};
use batlehub_web::services::{BannerService, ConfigReloadService};
use batlehub_web::{new_access_lock, openapi_spec, RateLimitService};

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
    HashToken {
        /// The plain-text token to hash.
        token: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::DumpSpec) => {
            let spec = openapi_spec();
            println!("{}", spec.to_pretty_json().expect("serialize openapi spec"));
            return Ok(());
        }
        Some(Command::HashToken { token }) => {
            println!("{}", batlehub_adapters::auth::hash_static_token(&token));
            return Ok(());
        }
        None => {}
    }

    let config_path = cli
        .config
        .or_else(|| std::env::var("BATLEHUB_CONFIG").ok())
        .unwrap_or_else(|| "config.toml".to_string());
    let config = batlehub_config::load(&config_path)
        .with_context(|| format!("loading config from '{config_path}'"))?;

    let prometheus_handle = PrometheusBuilder::new()
        .install_recorder()
        .context("installing Prometheus metrics recorder")?;

    let _tracer_provider = watcher::init_tracing(config.otel.as_ref());
    tracing::info!(config = %config_path, "batlehub starting");

    let repo = Arc::new(
        PgPackageRepository::new(
            &config.database.url,
            batlehub_adapters::db::postgres::PoolOptions {
                max_connections: config.database.max_connections,
                min_connections: config.database.min_connections,
                acquire_timeout_secs: config.database.acquire_timeout_secs,
            },
        )
        .await
        .context("connecting to database")?,
    );
    repo.run_migrations().await.context("running migrations")?;

    let storage = setup::initialize_storage(&config, repo.pool()).await?;
    let (mut auth_providers, oidc_sso_flows) = setup::initialize_auth_providers(&config).await?;
    let token_repo = repo.clone() as Arc<dyn UserTokenRepository>;
    setup::add_user_token_provider(&mut auth_providers, token_repo.clone());

    let cache = stores::create_cache_store(&config, repo.pool()).await?;
    let cargo_index_map = setup::build_initial_cargo_index_map(&config)?;

    let rate_limit_configs: HashMap<_, _> = config
        .registries
        .iter()
        .filter_map(|r| r.rate_limit.clone().map(|rl| (r.name.clone(), rl)))
        .collect();
    let rate_limit_store = stores::create_rate_limit_store(&config, repo.pool()).await?;
    let rate_limit_svc = Arc::new(RateLimitService::new(&rate_limit_configs, rate_limit_store));

    let registry_names: Vec<String> = config.registries.iter().map(|r| r.name.clone()).collect();
    let proxy_metrics = Arc::new(ProxyMetrics::new(&registry_names));
    let artifact_meta = Arc::new(PgArtifactMetaRepository::new(repo.pool()));
    let vuln_repo: Arc<dyn VulnerabilityRepository> =
        Arc::new(PgVulnerabilityRepository::new(repo.pool()));
    let admin_svc = Arc::new(
        AdminService::new(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>)
            .with_vulnerability_repo(Arc::clone(&vuln_repo)),
    );
    let local_registry_backend = Arc::new(PostgresLocalRegistry::new(repo.pool()));
    let quota_svc = Arc::new(builders::build_quota_service(
        repo.pool(),
        &config.registries,
    ));
    let ownership_store = Arc::new(PgOwnershipStore::new(repo.pool()))
        as Arc<dyn batlehub_core::ports::OwnershipPort>;
    let beta_channel_store: Arc<dyn BetaChannelPort> =
        Arc::new(PgBetaChannelStore::new(repo.pool()));
    let team_namespace_store: Arc<dyn batlehub_core::ports::TeamNamespacePort> =
        Arc::new(PgTeamNamespaceStore::new(repo.pool()));

    let (init_hot, init_access, registry_map, registry_mode_map, upstream_map) =
        hot_config::build_hot_bundle(
            &config,
            &beta_channel_store,
            &(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>),
            &vuln_repo,
        )?;
    let warming_clients: HashMap<String, Arc<dyn batlehub_core::ports::RegistryClient>> = init_hot
        .registries
        .iter()
        .map(|(k, v)| (k.clone(), Arc::clone(v)))
        .collect();
    let hot = new_hot_lock(init_hot);

    let sbom_svc = stores::build_sbom_service(repo.pool())?;
    let proxy_svc = Arc::new(ProxyService {
        hot: Arc::clone(&hot),
        storage: storage.clone(),
        cache: cache.clone(),
        repo: repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>,
        artifact_meta,
        metrics: Arc::clone(&proxy_metrics),
        sbom: Some(Arc::clone(&sbom_svc)),
    });

    let ip_block_store = stores::create_ip_block_store(&config, repo.pool()).await?;
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

    let warming_map =
        setup::build_warming_map(&config, &warming_clients, storage.clone(), repo.pool());
    let eviction_map = setup::build_eviction_map(&config, storage.clone(), repo.pool());
    let access_config = new_access_lock(init_access);

    let hot_reload_enabled = std::env::var("BATLEHUB_DISABLE_HOT_RELOAD")
        .map(|v| v != "1" && v.to_lowercase() != "true")
        .unwrap_or(true);
    let banner_store = stores::create_banner_store(&config, repo.pool()).await?;
    let banner_svc = Arc::new(BannerService::new(banner_store));
    let notification_store = stores::create_notification_store(repo.pool());
    let notification_svc =
        stores::build_notification_service(Arc::clone(&notification_store), &config.notifications);

    let hot_builder = hot_config::make_hot_builder(
        Arc::clone(&beta_channel_store),
        repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>,
        Arc::clone(&vuln_repo),
    );
    // Built once here so the same instance is shared with the reload service (for
    // hot-swapping) and registered as actix app_data below.
    let repo_signer_map = builders::build_repo_signer_map(&config)?;
    let reload_svc = Arc::new(ConfigReloadService::new(
        Arc::clone(&hot),
        Arc::clone(&access_config),
        registry_map.clone(),
        registry_mode_map.clone(),
        upstream_map.clone(),
        cargo_index_map.clone(),
        repo_signer_map.clone(),
        config_path.clone(),
        Some(repo.pool()),
        hot_reload_enabled,
        hot_builder,
        Some(Arc::clone(&banner_svc)),
    ));

    if hot_reload_enabled {
        watcher::spawn_config_watcher(config_path.clone(), Arc::clone(&reload_svc));
        tracing::info!("hot reload: enabled (watching {})", config_path);
    } else {
        tracing::info!("hot reload: disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
    }

    tracing::info!(
        addr = %format!("{}:{}", config.server.host, config.server.port),
        "listening"
    );
    watcher::spawn_startup_warming(&config, &warming_map);

    // Periodic SBOM re-check against the OSV vulnerability database.
    if let Some(vuln_cfg) = config.vulnerability_scan.as_ref().filter(|v| v.enabled) {
        let osv_client = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .build()
            .context("building OSV HTTP client")?;
        let scanner = Arc::new(OsvScanner::new(osv_client, vuln_cfg.osv_api_url.clone()));
        let scan_svc = Arc::new(VulnerabilityScanService::new(
            Arc::clone(&sbom_svc.repo),
            scanner,
            Arc::clone(&vuln_repo),
            vuln_cfg.batch_size as u64,
        ));
        watcher::spawn_periodic_vuln_scan(vuln_cfg.interval_secs, scan_svc);
        tracing::info!(
            interval_secs = vuln_cfg.interval_secs,
            "vuln-scan: periodic SBOM re-check enabled"
        );
    }

    server_factory::run_actix_server(server_factory::ServerParams {
        bind_addr: format!("{}:{}", config.server.host, config.server.port),
        static_dir: config.server.static_dir.clone(),
        cli_binary_path: config
            .server
            .cli_binary_path
            .as_deref()
            .map(std::path::PathBuf::from),
        cors_allowed_origins: config
            .server
            .cors_allowed_origins
            .clone()
            .unwrap_or_default(),
        db_pool: repo.pool(),
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        upstream_map,
        oidc_sso_flows,
        warming_map,
        eviction_map,
        proxy_metrics,
        prometheus_handle,
        sbom_svc,
        notification_svc,
        notification_store,
        notifications_config: config.notifications.clone(),
        local_svc,
        quota_svc,
        registry_mode_map,
        repo_signer_map,
        ip_block_store,
        beta_channel_store,
        team_namespace_store,
        ip_blocking_cfg,
        cargo_index_map,
        rate_limit_svc,
        auth_providers,
        reload_svc,
        banner_svc,
    })
    .await
}
