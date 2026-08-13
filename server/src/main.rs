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
    PgArtifactMetaRepository, PgBetaChannelStore, PgConfigChangeRepository, PgOwnershipStore,
    PgPackageRepository, PgStorageAdminRepository, PgTeamNamespaceStore, PgVulnerabilityRepository,
};
use batlehub_adapters::local_registry::PostgresLocalRegistry;
use batlehub_adapters::vulnerability::OsvScanner;
use batlehub_core::ports::{BetaChannelPort, UserTokenRepository, VulnerabilityRepository};
use batlehub_core::services::{
    new_hot_lock, AdminService, LocalRegistryService, ProxyMetrics, ProxyService,
    VulnerabilityScanService,
};
use batlehub_web::services::{BannerService, ConfigReloadParams, ConfigReloadService};
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

    // `/metrics` is unauthenticated and was, until RFC 0004, unconditional —
    // it publishes cache hit rates, per-registry pull volumes and upstream
    // latencies to anyone who can reach the port. Consulting config here is
    // what makes `[stats] metrics_enabled = false` mean anything, and what
    // makes the handler's existing "metrics not configured" branch reachable
    // in a real server for the first time rather than only in tests.
    let prometheus_handle = if config.stats.metrics_enabled {
        Some(
            PrometheusBuilder::new()
                .install_recorder()
                .context("installing Prometheus metrics recorder")?,
        )
    } else {
        tracing::info!("[stats] metrics_enabled = false — /metrics will report 503");
        None
    };

    let _tracer_provider = watcher::init_tracing(config.otel.as_ref());
    tracing::info!(config = %config_path, "batlehub starting");

    let repo = Arc::new(
        PgPackageRepository::new(
            &config.database.url,
            batlehub_adapters::db::packages::PoolOptions {
                max_connections: config.database.max_connections,
                min_connections: config.database.min_connections,
                acquire_timeout_secs: config.database.acquire_timeout_secs,
            },
        )
        .await
        .context("connecting to database")?,
    );
    repo.run_migrations().await.context("running migrations")?;
    stores::spawn_db_pool_gauge_sampler(repo.pool());

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

    // Shared between the hourly rollup writer and the admin read endpoint, so
    // both sides of the series are the same table.
    let stats_history: Arc<dyn batlehub_core::ports::StatsHistoryRepository> = Arc::new(
        batlehub_adapters::db::PgStatsHistoryRepository::new(repo.pool()),
    );
    let artifact_meta = Arc::new(PgArtifactMetaRepository::new(repo.pool()));
    let vuln_repo: Arc<dyn VulnerabilityRepository> =
        Arc::new(PgVulnerabilityRepository::new(repo.pool()));
    let admin_svc = Arc::new(
        AdminService::new(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>)
            .with_vulnerability_repo(Arc::clone(&vuln_repo)),
    );
    let local_registry_backend = Arc::new(PostgresLocalRegistry::new(repo.pool()));
    stores::spawn_pending_publish_cleanup(Arc::clone(&local_registry_backend));
    let quota_svc = Arc::new(builders::build_quota_service(
        repo.pool(),
        &config.registries,
    ));
    stores::spawn_quota_gauge_sampler(Arc::clone(&quota_svc));
    let ownership_store = Arc::new(PgOwnershipStore::new(repo.pool()))
        as Arc<dyn batlehub_core::ports::OwnershipPort>;
    let beta_channel_store: Arc<dyn BetaChannelPort> =
        Arc::new(PgBetaChannelStore::new(repo.pool()));
    let team_namespace_store: Arc<dyn batlehub_core::ports::TeamNamespacePort> =
        Arc::new(PgTeamNamespaceStore::new(repo.pool()));

    // Built before the hot bundle because `license_gate` reads the recorded
    // licence through it; `build_sbom_service` below wraps the same repository.
    let sbom_repo: Arc<dyn batlehub_core::ports::SbomRepository> =
        Arc::new(batlehub_adapters::db::PgSbomRepository::new(repo.pool()));

    let (init_hot, init_access, registry_map, registry_mode_map, upstream_map, vuln_db_map) =
        hot_config::build_hot_bundle(
            &config,
            &beta_channel_store,
            &(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>),
            &vuln_repo,
            &sbom_repo,
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
    let user_block_repo = stores::create_user_block_repository(repo.pool());
    let ip_blocking_cfg = config.ip_blocking.clone();
    // `[server].trusted_proxies`, falling back to the deprecated
    // `[ip_blocking].trusted_proxies`. Hot-reloadable, and handed to the reload
    // service below: it decides which peers may influence routing, so it has to
    // move in step with the host-routing table (which is hot-reloadable) or a
    // reload that turns host routing on would run under the startup policy. Each
    // request resolves its verdict once, so no in-flight request straddles two
    // policies.
    let proxy_trust = batlehub_web::ProxyTrust::from_config(config.effective_trusted_proxies());
    let registry_host_map = batlehub_web::RegistryHostMap::from_app_config(&config);
    let local_svc = Arc::new(LocalRegistryService {
        backend: local_registry_backend,
        storage: storage.clone(),
        hot: Arc::clone(&hot),
        quota: Some(Arc::clone(&quota_svc)),
        ownership: Some(ownership_store),
        team_namespace: Some(Arc::clone(&team_namespace_store)),
        sbom: Some(Arc::clone(&sbom_svc)),
        explore_cache: Some(Arc::clone(&admin_svc.explore_cache)),
        access_log: Some(repo.clone() as Arc<dyn batlehub_core::ports::PackageRepository>),
    });

    let warm_coordinator = stores::create_warm_coordinator(&config).await?;
    let warming_map = setup::build_warming_map(
        &config,
        &warming_clients,
        storage.clone(),
        repo.pool(),
        warm_coordinator,
        Arc::clone(&proxy_metrics),
    );
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
        Arc::clone(&sbom_repo),
    );
    // Built once here so the same instance is shared with the reload service (for
    // hot-swapping) and registered as actix app_data below.
    let repo_signer_map = builders::build_repo_signer_map(&config)?;
    let config_change_repo: Arc<dyn batlehub_core::ports::ConfigChangeRepository> =
        Arc::new(PgConfigChangeRepository::new(repo.pool()));
    let storage_admin_repo: Arc<dyn batlehub_core::ports::StorageAdminRepository> =
        Arc::new(PgStorageAdminRepository::new(repo.pool()));
    let reload_svc = Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot: Arc::clone(&hot),
        access: Arc::clone(&access_config),
        registry_map: registry_map.clone(),
        registry_mode_map: registry_mode_map.clone(),
        upstream_map: upstream_map.clone(),
        cargo_index_map: cargo_index_map.clone(),
        repo_signer_map: repo_signer_map.clone(),
        vuln_db_map: vuln_db_map.clone(),
        registry_host_map: registry_host_map.clone(),
        // The same handle wrapped into the host-routing middleware and registered
        // as `app_data` below — clones share a lock, which is what lets a reload
        // reach the policy those two actually read.
        proxy_trust: proxy_trust.clone(),
        config_path: config_path.clone(),
        config_change_repo: Some(Arc::clone(&config_change_repo)),
        hot_reload_enabled,
        builder: hot_builder,
        banner: Some(Arc::clone(&banner_svc)),
    }));

    // Seed the warning store from the config we booted with (this also logs each
    // one). Reloads refresh it themselves.
    reload_svc.set_warnings(config.warnings());

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

    // Hourly cache-statistics rollup, so the dashboard's trend survives a
    // deploy (RFC 0004 §2.3). `history_enabled = false` restores the previous
    // behaviour: counters since this process started, and nothing older.
    if config.stats.history_enabled {
        let rollup = Arc::new(batlehub_core::services::StatsRollupService::new(
            Arc::clone(&proxy_metrics),
            Arc::clone(&stats_history),
            config.stats.history_retention_days,
        ));
        watcher::spawn_stats_rollup(rollup, Arc::clone(&proxy_svc), registry_names.clone());
        tracing::info!(
            retention_days = config.stats.history_retention_days,
            "stats-rollup: hourly cache-statistics history enabled"
        );
    } else {
        tracing::info!("[stats] history_enabled = false — no rollup recorded");
    }

    // Periodic SBOM re-check against the OSV vulnerability database.
    if let Some(vuln_cfg) = config.vulnerability_scan.as_ref().filter(|v| v.enabled) {
        let osv_client = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .connect_timeout(std::time::Duration::from_secs(30))
            .timeout(std::time::Duration::from_secs(60))
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
        vuln_db_map,
        oidc_sso_flows,
        warming_map,
        eviction_map,
        proxy_metrics,
        prometheus_handle,
        stats_history,
        sbom_svc,
        notification_svc,
        notification_store,
        notifications_config: config.notifications.clone(),
        local_svc,
        quota_svc,
        registry_mode_map,
        repo_signer_map,
        ip_block_store,
        user_block_repo,
        beta_channel_store,
        team_namespace_store,
        ip_blocking_cfg,
        proxy_trust,
        registry_host_map,
        cargo_index_map,
        rate_limit_svc,
        auth_providers,
        reload_svc,
        banner_svc,
        storage_admin_repo,
    })
    .await
}
