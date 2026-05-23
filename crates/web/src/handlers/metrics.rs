use actix_web::{HttpResponse, Responder, get, web};
use metrics_exporter_prometheus::PrometheusHandle;

/// Prometheus metrics endpoint — scraped by Prometheus or compatible tools.
/// No authentication required.
#[get("/metrics")]
pub async fn prometheus_metrics(handle: Option<web::Data<PrometheusHandle>>) -> impl Responder {
    match handle {
        Some(h) => HttpResponse::Ok()
            .content_type("text/plain; version=0.0.4; charset=utf-8")
            .body(h.render()),
        None => HttpResponse::ServiceUnavailable().body("metrics not configured"),
    }
}
