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

// ── /api/v1/admin/stats/history ───────────────────────────────────────────────

use batlehub_adapters::in_memory::InMemoryStatsHistory;
use batlehub_core::ports::{StatsHistoryRepository, StatsRollupRow};
use chrono::{Duration, Utc};

/// A rollup row `hours_ago` hours in the past.
fn rollup(registry: &str, hours_ago: i64, hits: u64, misses: u64) -> StatsRollupRow {
    StatsRollupRow {
        registry: registry.to_owned(),
        window_start: batlehub_core::services::hour_start(Utc::now() - Duration::hours(hours_ago)),
        hits,
        misses,
        listing_reads: 0,
        cached_bytes: 1_024,
    }
}

#[actix_web::test]
async fn stats_history_requires_admin_role() {
    let app = make_app_with_stats_history(InMemoryStatsHistory::new()).await;

    for header in [None, Some(bearer(USER_TOKEN))] {
        let mut req = TestRequest::get().uri("/api/v1/admin/stats/history");
        if let Some(h) = header {
            req = req.insert_header(("Authorization", h));
        }
        let resp = call_service(&app, req.to_request()).await;
        assert_eq!(resp.status(), 403);
    }
}

#[actix_web::test]
async fn stats_history_sums_the_window_and_groups_by_registry() {
    let history = InMemoryStatsHistory::new();
    history
        .append(&[
            rollup("npm", 2, 8, 2),
            rollup("npm", 1, 6, 4),
            rollup("cargo", 1, 10, 0),
        ])
        .await
        .unwrap();
    let app = make_app_with_stats_history(history).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/stats/history?window=30d")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;

    assert_eq!(body["window_days"], 30);
    assert_eq!(body["trend"]["hits"], 24);
    assert_eq!(body["trend"]["misses"], 6);
    assert_eq!(body["trend"]["hit_rate"], 0.8);

    let per = body["per_registry"].as_array().unwrap();
    assert_eq!(per.len(), 2);
    assert_eq!(per[0]["registry"], "cargo", "sorted by registry");
    assert_eq!(per[1]["registry"], "npm");
    assert_eq!(per[1]["points"].as_array().unwrap().len(), 2);
}

/// The question `/api/v1/admin/stats` structurally cannot answer: better or
/// worse than before (RFC 0004 §2.3).
#[actix_web::test]
async fn stats_history_compares_the_window_with_the_one_before_it() {
    let history = InMemoryStatsHistory::new();
    history
        .append(&[
            // Previous 7-day window: 50% hit rate.
            rollup("npm", 24 * 10, 5, 5),
            // Current 7-day window: 90%.
            rollup("npm", 2, 9, 1),
        ])
        .await
        .unwrap();
    let app = make_app_with_stats_history(history).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/stats/history?window=7d")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    let body: Value = read_body_json(resp).await;

    assert_eq!(body["trend"]["hit_rate"], 0.9);
    assert_eq!(body["trend"]["previous_hit_rate"], 0.5);
    assert!(
        (body["trend"]["delta"].as_f64().unwrap() - 0.4).abs() < 1e-9,
        "the delta is computed server-side so both halves are derived the same way"
    );
}

#[actix_web::test]
async fn stats_history_distinguishes_no_traffic_from_a_zero_rate() {
    let app = make_app_with_stats_history(InMemoryStatsHistory::new()).await;
    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/stats/history")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    let body: Value = read_body_json(resp).await;

    assert!(body["trend"]["hit_rate"].is_null(), "no traffic is not 0%");
    assert!(body["trend"]["previous_hit_rate"].is_null());
    assert!(body["trend"]["delta"].is_null(), "nothing to subtract");
    assert!(body["per_registry"].as_array().unwrap().is_empty());
}

#[actix_web::test]
async fn stats_history_excludes_rows_outside_the_window() {
    let history = InMemoryStatsHistory::new();
    history
        .append(&[rollup("npm", 24 * 40, 100, 0), rollup("npm", 1, 1, 1)])
        .await
        .unwrap();
    let app = make_app_with_stats_history(history).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/stats/history?window=7d")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    let body: Value = read_body_json(resp).await;
    assert_eq!(
        body["trend"]["hits"], 1,
        "the 40-day-old row is not in a 7-day window"
    );
}

#[actix_web::test]
async fn stats_history_clamps_an_absurd_window_rather_than_failing() {
    let app = make_app_with_stats_history(InMemoryStatsHistory::new()).await;
    for (query, expected) in [
        ("window=0", 1),
        ("window=100000", 365),
        ("window=nonsense", 30),
    ] {
        let resp = call_service(
            &app,
            TestRequest::get()
                .uri(&format!("/api/v1/admin/stats/history?{query}"))
                .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 200, "{query}");
        let body: Value = read_body_json(resp).await;
        assert_eq!(body["window_days"], expected, "{query}");
    }
}
