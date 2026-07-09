use std::sync::Arc;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::http;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{trace as sdktrace, Resource};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use batlehub_config::schema::{AppConfig, OtelConfig};
use batlehub_web::handlers::back_office::ops::warming::WarmingServiceMap;
use batlehub_web::services::ConfigReloadService;

// ── CORS ──────────────────────────────────────────────────────────────────────

pub(super) fn build_cors(allowed_origins: &[String]) -> Cors {
    let base = Cors::default()
        .allowed_methods(vec!["GET", "POST", "PUT", "HEAD", "OPTIONS", "DELETE"])
        .allowed_headers(vec![
            http::header::AUTHORIZATION,
            http::header::CONTENT_TYPE,
            http::header::ACCEPT,
        ])
        .max_age(3600);
    if allowed_origins.is_empty() {
        base.allow_any_origin()
    } else {
        allowed_origins
            .iter()
            .fold(base, |c, origin| c.allowed_origin(origin))
    }
}

// ── Startup warming ───────────────────────────────────────────────────────────

pub(super) fn spawn_startup_warming(config: &AppConfig, warming_map: &WarmingServiceMap) {
    for reg in &config.registries {
        if reg.cache.warm_packages.is_empty() && reg.cache.warm_paths.is_empty() {
            continue;
        }
        if let Some(svc) = warming_map.get(&reg.name) {
            let svc = Arc::clone(svc);
            let packages = reg.cache.warm_packages.clone();
            let paths = reg.cache.warm_paths.clone();
            let name = reg.name.clone();
            tokio::spawn(async move {
                tracing::info!(registry = %name, "warming: startup warming started");
                let mut report = svc.warm_all(&packages).await;
                report += svc.warm_all_paths(&paths).await;
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

// ── Periodic vulnerability scan ─────────────────────────────────────────────────

/// Spawn a background task that re-checks all cached SBOMs against the OSV
/// vulnerability database: once shortly after startup, then every
/// `interval_secs`. Mirrors `spawn_startup_warming` — a detached `tokio::spawn`
/// that logs a summary per run.
pub(super) fn spawn_periodic_vuln_scan(
    interval_secs: u64,
    scan_svc: Arc<batlehub_core::services::VulnerabilityScanService>,
) {
    let period = Duration::from_secs(interval_secs.max(1));
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            tracing::info!("vuln-scan: starting periodic SBOM re-check");
            match scan_svc.scan_all().await {
                Ok(report) => tracing::info!(
                    scanned = report.scanned,
                    findings = report.findings,
                    errors = report.errors,
                    "vuln-scan: periodic re-check complete"
                ),
                Err(e) => tracing::warn!(error = %e, "vuln-scan: periodic re-check failed"),
            }
        }
    });
}

// ── Config file watcher ───────────────────────────────────────────────────────

/// OS-thread body: owns the blocking `notify` watcher and forwards change events
/// to the async side via `event_tx`. Exits when `event_tx` is closed.
fn run_watcher_thread(config_path: String, event_tx: tokio::sync::mpsc::UnboundedSender<()>) {
    use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc::channel;

    let (notify_tx, notify_rx) = channel();
    let mut watcher = match RecommendedWatcher::new(
        notify_tx,
        NotifyConfig::default().with_poll_interval(Duration::from_secs(2)),
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
        tracing::error!(error = %e, "config file watcher: failed to watch {config_path}");
        return;
    }
    tracing::info!(path = %config_path, "config file watcher started");

    loop {
        match notify_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(_) => {
                while notify_rx.try_recv().is_ok() {}
                if event_tx.send(()).is_err() {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if event_tx.is_closed() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    tracing::info!("config file watcher stopped");
}

/// Async task body: receives file-change notifications and triggers config reloads.
async fn run_reload_task(
    reload_svc: Arc<ConfigReloadService>,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    use batlehub_web::services::ReloadSource;

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
    reload_svc.expire_pending_if_stale();
    tracing::debug!("config reload task exiting");
}

pub(super) fn spawn_config_watcher(config_path: String, reload_svc: Arc<ConfigReloadService>) {
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    std::thread::Builder::new()
        .name("config-watcher".to_owned())
        .spawn(move || run_watcher_thread(config_path, event_tx))
        .expect("failed to spawn config-watcher thread");

    tokio::spawn(run_reload_task(reload_svc, event_rx));
}

// ── Tracing ───────────────────────────────────────────────────────────────────

/// Initialise tracing. Returns the `TracerProvider` when OTLP is configured
/// so the caller can keep it alive for the process lifetime and flush on exit.
pub(super) fn init_tracing(otel_cfg: Option<&OtelConfig>) -> Option<sdktrace::SdkTracerProvider> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let (otel_layer, provider) = match otel_cfg {
        Some(cfg) => match build_otlp_provider(cfg) {
            Ok(p) => {
                use opentelemetry::trace::TracerProvider as _;
                let tracer = p.tracer(cfg.service_name.clone());
                let layer = tracing_opentelemetry::layer().with_tracer(tracer);
                (Some(layer), Some(p))
            }
            Err(e) => {
                eprintln!("WARN: failed to build OTLP exporter: {e}");
                (None, None)
            }
        },
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

    Ok(sdktrace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(resource)
        .build())
}
