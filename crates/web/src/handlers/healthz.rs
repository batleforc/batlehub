use std::sync::Arc;

use actix_web::{HttpResponse, Responder, get, http::StatusCode, web};
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
}

/// Infrastructure health check — verifies DB and storage connectivity.
/// Unauthenticated; intended for Kubernetes liveness/readiness probes.
#[get("/healthz")]
pub async fn healthz(
    pool: Option<web::Data<PgPool>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
) -> impl Responder {
    let db: &'static str = match pool {
        Some(p) => match sqlx::query("SELECT 1").execute(p.get_ref()).await {
            Ok(_) => STATUS_OK,
            Err(e) => {
                tracing::warn!(error = %e, "healthz: database check failed");
                STATUS_ERROR
            }
        },
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
    let status = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };

    HttpResponse::build(status).json(HealthResponse { ok, db, storage })
}
