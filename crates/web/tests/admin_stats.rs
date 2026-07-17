//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_core::services::ProxyMetrics;

// ── /api/v1/admin/stats ───────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_stats_requires_admin_role() {
    let app = make_app(InMemoryRepo::new()).await;

    let req = TestRequest::get().uri("/api/v1/admin/stats").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "anonymous must be denied");

    let req = TestRequest::get()
        .uri("/api/v1/admin/stats")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403, "user role must be denied");
}

#[actix_web::test]
async fn admin_stats_returns_zero_counts_initially() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/stats")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["aggregate"]["artifact_hits"], 0);
    assert_eq!(body["aggregate"]["artifact_misses"], 0);
    assert!(
        body["aggregate"]["hit_rate"].is_null(),
        "hit_rate must be null when there are no requests"
    );
    assert!(body["since_startup"].is_string());
    assert!(body["per_registry"].is_array());
}

#[actix_web::test]
async fn admin_stats_reflects_counter_updates() {
    let proxy_metrics = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
    let app = make_app_ext(InMemoryRepo::new(), proxy_metrics.clone()).await;

    proxy_metrics.record_artifact_hit("npm");
    proxy_metrics.record_artifact_hit("npm");
    proxy_metrics.record_artifact_miss("npm");

    let req = TestRequest::get()
        .uri("/api/v1/admin/stats")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["aggregate"]["artifact_hits"], 2);
    assert_eq!(body["aggregate"]["artifact_misses"], 1);

    let hit_rate = body["aggregate"]["hit_rate"]
        .as_f64()
        .expect("hit_rate must be present");
    assert!(
        (hit_rate - 2.0 / 3.0).abs() < 1e-9,
        "expected hit_rate ≈ 0.667, got {hit_rate}"
    );

    let per_npm = body["per_registry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["registry"] == "npm")
        .expect("npm entry must be present");
    assert_eq!(per_npm["artifact_hits"], 2);
    assert_eq!(per_npm["artifact_misses"], 1);
    assert_eq!(per_npm["upstream_degraded"], false);
}

#[actix_web::test]
async fn admin_stats_flags_upstream_degraded_after_repeated_errors() {
    let proxy_metrics = Arc::new(ProxyMetrics::new(&["npm".to_owned()]));
    let app = make_app_ext(InMemoryRepo::new(), proxy_metrics.clone()).await;

    for _ in 0..20 {
        proxy_metrics.record_upstream_outcome("npm", false);
    }

    let req = TestRequest::get()
        .uri("/api/v1/admin/stats")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    let body: Value = read_body_json(resp).await;

    let per_npm = body["per_registry"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["registry"] == "npm")
        .expect("npm entry must be present");
    assert_eq!(per_npm["upstream_degraded"], true);
    assert!(per_npm["upstream_error_rate"].as_f64().unwrap() > 0.5);
}
