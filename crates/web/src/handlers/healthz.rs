use std::sync::Arc;

use actix_web::{get, http::StatusCode, web, HttpResponse, Responder};
use serde::Serialize;
use sqlx::PgPool;

use batlehub_core::services::ProxyService;

const STATUS_OK: &str = "ok";
const STATUS_ERROR: &str = "error";
const STATUS_UNCONFIGURED: &str = "unconfigured";

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    db: &'static str,
    storage: &'static str,
    /// The running server's version (`CARGO_PKG_VERSION`), so operators and
    /// the UI can surface what's deployed without a separate authenticated call.
    version: &'static str,
}

/// Infrastructure health check — verifies DB and storage connectivity, and
/// reports the running version. Unauthenticated; intended for Kubernetes
/// liveness/readiness probes as well as UI/CLI version display.
#[get("/healthz")]
pub async fn healthz(
    pool: Option<web::Data<PgPool>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> impl Responder {
    let db: &'static str = match pool {
        Some(p) => {
            let result = batlehub_adapters::db::timed_query(
                "healthz_ping",
                sqlx::query("SELECT 1").execute(p.get_ref()),
            )
            .await;
            match result {
                Ok(_) => STATUS_OK,
                Err(e) => {
                    tracing::warn!(error = %e, "healthz: database check failed");
                    STATUS_ERROR
                }
            }
        }
        None => STATUS_UNCONFIGURED,
    };

    let storage: &'static str = match proxy_svc.storage.exists("__healthz__").await {
        Ok(_) => STATUS_OK,
        Err(e) => {
            tracing::warn!(error = %e, "healthz: storage check failed");
            STATUS_ERROR
        }
    };

    let ok = db != STATUS_ERROR && storage != STATUS_ERROR;
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    HttpResponse::build(status).json(HealthResponse {
        ok,
        db,
        storage,
        version: env!("CARGO_PKG_VERSION"),
    })
}
