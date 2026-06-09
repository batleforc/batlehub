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
    UserTokenRepository,
};
use batlehub_core::services::{
    AdminService, LocalRegistryService, ProxyMetrics, ProxyService, QuotaService, SbomService,
};
use batlehub_web::handlers::back_office::warming::WarmingServiceMap;
use batlehub_web::services::{BannerService, ConfigReloadService, NotificationService};
use batlehub_web::{
    configure_app, healthz, prometheus_metrics, AccessConfigLock, ApiDoc, CargoIndexMap,
    CliBinaryPath, IpBlockMiddlewareFactory, RateLimitMiddlewareFactory, RateLimitService,
    RegistryMap, RegistryModeMap, UpstreamMap,
};

// ── Tracing span builder ──────────────────────────────────────────────────────

pub(super) struct BatleHubSpanBuilder;

impl RootSpanBuilder for BatleHubSpanBuilder {
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
    pub registry_map: RegistryMap,
    pub upstream_map: UpstreamMap,
    pub oidc_sso_flows: Vec<OidcSsoFlow>,
    pub warming_map: WarmingServiceMap,
    pub proxy_metrics: Arc<ProxyMetrics>,
    pub prometheus_handle: PrometheusHandle,
    pub sbom_svc: Arc<SbomService>,
    pub notification_svc: Option<Arc<NotificationService>>,
    pub notification_store: Arc<dyn NotificationPort>,
    pub notifications_config: Option<NotificationsConfig>,
    pub local_svc: Arc<LocalRegistryService>,
    pub quota_svc: Arc<QuotaService>,
    pub registry_mode_map: RegistryModeMap,
    pub ip_block_store: Arc<dyn IpBlockStore>,
    pub beta_channel_store: Arc<dyn BetaChannelPort>,
    pub team_namespace_store: Arc<dyn TeamNamespacePort>,
    pub ip_blocking_cfg: Option<IpBlockingConfig>,
    pub cargo_index_map: CargoIndexMap,
    pub rate_limit_svc: Arc<RateLimitService>,
    pub auth_providers: Vec<Arc<dyn AuthProvider>>,
    pub reload_svc: Arc<ConfigReloadService>,
    pub banner_svc: Arc<BannerService>,
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
        proxy_metrics,
        prometheus_handle,
        sbom_svc,
        notification_svc,
        notification_store,
        notifications_config,
        local_svc,
        quota_svc,
        registry_mode_map,
        ip_block_store,
        beta_channel_store,
        team_namespace_store,
        ip_blocking_cfg,
        cargo_index_map,
        rate_limit_svc,
        auth_providers,
        reload_svc,
        banner_svc,
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
            Arc::clone(&proxy_metrics),
            Some(prometheus_handle.clone()),
            Some(sbom_svc.clone()),
            notification_svc.clone(),
            Arc::clone(&notification_store),
            notifications_config.clone(),
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
            .app_data(web::Data::new(local_svc.clone()))
            .app_data(web::Data::new(Arc::clone(&quota_svc)))
            .app_data(web::Data::new(registry_mode_map.clone()))
            .app_data(web::Data::new(Arc::clone(&ip_block_store)))
            .app_data(web::Data::new(Arc::clone(&beta_channel_store)))
            .app_data(web::Data::new(Arc::clone(&team_namespace_store)))
            .app_data(web::Data::new(Arc::clone(&reload_svc)))
            .app_data(web::Data::new(Arc::clone(&banner_svc)))
            .service(prometheus_metrics)
            .service(healthz);

        if let Some(path) = cli_binary_path_inner {
            app = app.app_data(web::Data::new(CliBinaryPath(path)));
        }

        let cors = crate::watcher::build_cors(&cors_allowed_origins);
        let enabled = ip_blocking_cfg.as_ref().is_some_and(|c| c.enabled);
        let ip_block_cfg_for_mw = ip_blocking_cfg.clone().unwrap_or_default();

        app.wrap(TracingLogger::<BatleHubSpanBuilder>::new())
            .wrap(RateLimitMiddlewareFactory::new(rate_limit_svc.clone()))
            .wrap(batlehub_web::AuthMiddlewareFactory::new(
                auth_providers.clone(),
            ))
            .wrap(cors)
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

    if let Some(svc) = &notification_svc_for_shutdown {
        svc.shutdown().await;
    }

    Ok(())
}
