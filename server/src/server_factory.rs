use std::path::PathBuf;
use std::sync::Arc;

use actix_web::{web, App, HttpServer};
use anyhow::Context;
use metrics_exporter_prometheus::PrometheusHandle;
use tracing_actix_web::{DefaultRootSpanBuilder, RootSpanBuilder, TracingLogger};
use utoipa::OpenApi as _;
use utoipa_actix_web::AppExt;

use batlehub_adapters::auth::OidcSsoFlow;
use batlehub_config::schema::{IpBlockingConfig, NotificationsConfig};
use batlehub_core::ports::{
    AuthProvider, BetaChannelPort, IpBlockStore, NotificationPort, TeamNamespacePort,
    UserBlockRepository, UserTokenRepository,
};
use batlehub_core::services::{
    AdminService, LocalRegistryService, ProxyMetrics, ProxyService, QuotaService, SbomService,
};
use batlehub_web::handlers::back_office::ops::eviction::EvictionServiceMap;
use batlehub_web::handlers::back_office::ops::warming::WarmingServiceMap;
use batlehub_web::services::{BannerService, ConfigReloadService, NotificationService};
use batlehub_web::{
    configure_app, healthz, livez, prometheus_metrics, security_headers, AccessConfigLock, ApiDoc,
    CargoIndexMap, CliBinaryPath, HostRoutingMiddlewareFactory, IpBlockMiddlewareFactory,
    ProxyTrust, RateLimitMiddlewareFactory, RateLimitService, RegistryHostMap, RegistryMap,
    RegistryModeMap, SumDbMap, UpstreamMap, UserBlockMiddlewareFactory, VulnDbMap,
};

// ── Tracing span builder ──────────────────────────────────────────────────────

pub(super) struct BatleHubSpanBuilder;

impl RootSpanBuilder for BatleHubSpanBuilder {
    // `root_span!` expands to `ConnectionInfo::scheme`/`::host` calls, which
    // `clippy.toml` disallows so no request-handling code reads a forwarded
    // header without going through `proxy_trust`. This is third-party macro
    // output and it only labels a tracing span — a spoofed host here mislabels a
    // trace, it does not decide routing, URLs or a ban. Nothing else in the
    // workspace carries this allow; see clippy.toml for why.
    #[allow(clippy::disallowed_methods)]
    fn on_request_start(request: &actix_web::dev::ServiceRequest) -> tracing::Span {
        tracing_actix_web::root_span!(level = tracing::Level::INFO, request)
    }

    fn on_request_end<B: actix_web::body::MessageBody>(
        span: tracing::Span,
        outcome: &anyhow::Result<actix_web::dev::ServiceResponse<B>, actix_web::Error>,
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

// ── Server startup params ─────────────────────────────────────────────────────

pub(super) struct ServerParams {
    pub bind_addr: String,
    pub static_dir: Option<String>,
    pub cli_binary_path: Option<PathBuf>,
    pub cors_allowed_origins: Vec<String>,
    pub db_pool: sqlx::PgPool,
    pub proxy_svc: Arc<ProxyService>,
    pub admin_svc: Arc<AdminService>,
    pub token_repo: Arc<dyn UserTokenRepository>,
    pub access_config: AccessConfigLock,
    /// `[search] readmes`, shared with the reload path so turning prose search
    /// off takes effect without a restart (RFC 0007-bis §4.1).
    pub search_config: batlehub_web::SearchConfigLock,
    pub registry_map: RegistryMap,
    pub upstream_map: UpstreamMap,
    pub oidc_sso_flows: Vec<OidcSsoFlow>,
    pub warming_map: WarmingServiceMap,
    pub eviction_map: EvictionServiceMap,
    pub proxy_metrics: Arc<ProxyMetrics>,
    /// `None` when `[stats] metrics_enabled = false`: the recorder is never
    /// installed, and `/metrics` answers 503 rather than publishing.
    pub prometheus_handle: Option<PrometheusHandle>,
    pub sbom_svc: Arc<SbomService>,
    pub notification_svc: Option<Arc<NotificationService>>,
    pub notification_store: Arc<dyn NotificationPort>,
    pub notifications_config: Option<NotificationsConfig>,
    pub local_svc: Arc<LocalRegistryService>,
    pub quota_svc: Arc<QuotaService>,
    pub stats_history: Arc<dyn batlehub_core::ports::StatsHistoryRepository>,
    pub registry_mode_map: RegistryModeMap,
    pub repo_signer_map: batlehub_web::RepoSignerMap,
    pub ip_block_store: Arc<dyn IpBlockStore>,
    pub user_block_repo: Arc<dyn UserBlockRepository>,
    pub beta_channel_store: Arc<dyn BetaChannelPort>,
    pub team_namespace_store: Arc<dyn TeamNamespacePort>,
    pub ip_blocking_cfg: Option<IpBlockingConfig>,
    /// Resolved `[server].trusted_proxies` (or the deprecated
    /// `[ip_blocking]` fallback). Registered as `app_data` so the middleware
    /// stack and the outbound URL helper share one verdict per request.
    pub proxy_trust: ProxyTrust,
    /// `host -> registry` routing table for host-based ingress. Empty when the
    /// feature is unconfigured, which makes the middleware a no-op.
    pub registry_host_map: RegistryHostMap,
    pub cargo_index_map: CargoIndexMap,
    pub vuln_db_map: VulnDbMap,
    pub sumdb_map: SumDbMap,
    pub rate_limit_svc: Arc<RateLimitService>,
    pub auth_providers: Vec<Arc<dyn AuthProvider>>,
    pub reload_svc: Arc<ConfigReloadService>,
    pub banner_svc: Arc<BannerService>,
    pub storage_admin_repo: Arc<dyn batlehub_core::ports::StorageAdminRepository>,
}

// ── HTTP server ───────────────────────────────────────────────────────────────

pub(super) async fn run_actix_server(p: ServerParams) -> anyhow::Result<()> {
    let ServerParams {
        bind_addr,
        static_dir,
        cli_binary_path,
        cors_allowed_origins,
        db_pool,
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
        notifications_config,
        local_svc,
        quota_svc,
        stats_history,
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
        vuln_db_map,
        sumdb_map,
        rate_limit_svc,
        auth_providers,
        reload_svc,
        banner_svc,
        storage_admin_repo,
        search_config,
    } = p;

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
            eviction_map.clone(),
            Arc::clone(&proxy_metrics),
            prometheus_handle.clone(),
            Some(sbom_svc.clone()),
            notification_svc.clone(),
            Arc::clone(&notification_store),
            notifications_config.clone(),
            Some(Arc::clone(&storage_admin_repo)),
            search_config.clone(),
        );
        let static_dir_inner = static_dir.clone();
        let cli_binary_path_inner = cli_binary_path.clone();

        let (app, openapi) = App::new()
            .into_utoipa_app()
            .openapi(ApiDoc::openapi())
            .configure(configure)
            .split_for_parts();

        let mut app = app
            .app_data(web::Data::new(cargo_index_map.clone()))
            .app_data(web::Data::new(vuln_db_map.clone()))
            .app_data(web::Data::new(sumdb_map.clone()))
            .app_data(web::Data::new(local_svc.clone()))
            .app_data(web::Data::new(Arc::clone(&quota_svc)))
            .app_data(web::Data::new(Arc::clone(&stats_history)))
            .app_data(web::Data::new(registry_mode_map.clone()))
            .app_data(web::Data::new(repo_signer_map.clone()))
            .app_data(web::Data::new(Arc::clone(&ip_block_store)))
            .app_data(web::Data::new(Arc::clone(&user_block_repo)))
            .app_data(web::Data::new(Arc::clone(&beta_channel_store)))
            .app_data(web::Data::new(Arc::clone(&team_namespace_store)))
            .app_data(web::Data::new(Arc::clone(&reload_svc)))
            .app_data(web::Data::new(Arc::clone(&banner_svc)))
            .app_data(web::Data::new(proxy_trust.clone()))
            .app_data(web::Data::new(registry_host_map.clone()))
            .service(prometheus_metrics)
            .service(healthz)
            .service(livez);

        if let Some(path) = cli_binary_path_inner {
            app = app.app_data(web::Data::new(CliBinaryPath(path)));
        }

        let cors = crate::watcher::build_cors(&cors_allowed_origins);
        let enabled = ip_blocking_cfg.as_ref().is_some_and(|c| c.enabled);
        let ip_block_cfg_for_mw = ip_blocking_cfg.clone().unwrap_or_default();

        app.wrap(TracingLogger::<BatleHubSpanBuilder>::new())
            .wrap(RateLimitMiddlewareFactory::new(rate_limit_svc.clone()))
            .wrap(UserBlockMiddlewareFactory::new(Arc::clone(
                &user_block_repo,
            )))
            .wrap(batlehub_web::AuthMiddlewareFactory::new(
                auth_providers.clone(),
            ))
            .wrap(cors)
            .wrap(actix_web::middleware::Condition::new(
                enabled,
                IpBlockMiddlewareFactory::new(Arc::clone(&ip_block_store), ip_block_cfg_for_mw),
            ))
            // Outside the IP-block layer so its 403 — and the rate limiter's 429,
            // and anything the static-file service returns — carry the baseline
            // headers too, not just handler responses.
            .wrap(security_headers())
            // Outermost, so the URI rewrite lands before route matching and the
            // proxy-trust verdict before anything that reads a forwarded header.
            // `.wrap` builds inside-out, so this must stay the last call.
            .wrap(HostRoutingMiddlewareFactory::new(
                registry_host_map.clone(),
                proxy_trust.clone(),
            ))
            .service(batlehub_web::scalar(openapi))
            .configure(move |cfg| {
                if let Some(ref dir) = static_dir_inner {
                    // Still no CSP *header* here, for the two reasons that have
                    // not changed: it cannot be global — the Scalar API-docs page
                    // loads its bundle from a CDN, so `script-src 'self'` would
                    // break `/scalar` — and `actix_files::Files` is not a
                    // `ServiceFactory`, so it cannot be wrapped individually
                    // either. The SPA carries its own policy in a
                    // `<meta http-equiv>` tag, generated at build time by
                    // `ui/build/csp.ts` so `connect-src` can follow the configured
                    // API origin. `frame-ancestors` is ignored in meta form, which
                    // is why `security_headers()` sends `X-Frame-Options: DENY`.
                    //
                    // What *is* new: the document itself is served by
                    // `configure_spa` rather than by `Files`, so the built policy
                    // can be narrowed to the running config on the way out — see
                    // `crates/web/src/spa.rs` for why that narrowing can only ever
                    // subtract. It must be registered **first**: `Files` mounted
                    // at "/" would otherwise answer `/` and `/index.html` itself,
                    // straight off disk, and which policy a reader got would
                    // depend on the URL they arrived by.
                    cfg.app_data(actix_web::web::Data::new(batlehub_web::SpaDir(
                        std::path::PathBuf::from(dir),
                    )));
                    batlehub_web::configure_spa(cfg);
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

    if let Some(svc) = &notification_svc_for_shutdown {
        svc.shutdown().await;
    }

    Ok(())
}
