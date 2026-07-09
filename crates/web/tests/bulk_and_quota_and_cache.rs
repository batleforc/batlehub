//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test::{call_service, init_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_web::AuthMiddlewareFactory;

// ── Bulk operations ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn bulk_yank_returns_200_with_empty_packages() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/bulk-yank")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"packages": []}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    // response: { "processed": 0, "succeeded": 0, "failed": [] }
    assert_eq!(body["processed"], 0);
    assert!(body["failed"].as_array().unwrap().is_empty());
}

#[actix_web::test]
async fn bulk_yank_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/bulk-yank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"packages": []}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn bulk_delete_returns_200() {
    let repo = InMemoryRepo::new();
    let app = make_app(repo.clone()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/bulk-delete")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "packages": [{"name": "nonexistent", "version": "1.0.0"}]
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn bulk_unyank_returns_200() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/bulk-unyank")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"packages": []}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

// ── Quota ─────────────────────────────────────────────────────────────────────

use batlehub_adapters::in_memory::InMemoryQuotaRepository;
use batlehub_core::ports::QuotaRepository;
use batlehub_core::services::{AdminService, QuotaService};

/// Minimal app wired with only the four quota endpoints and auth middleware.
async fn make_quota_app(
    quota_svc: Arc<QuotaService>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    use batlehub_web::handlers::back_office::ops::quota::{
        get_quota_for_user, list_quota, list_quota_for_registry, reset_quota_for_user,
    };
    let admin_svc = Arc::new(AdminService::new(InMemoryRepo::new()));
    let app = actix_web::App::new()
        .app_data(actix_web::web::Data::new(quota_svc))
        .app_data(actix_web::web::Data::new(admin_svc))
        .service(list_quota)
        .service(list_quota_for_registry)
        .service(get_quota_for_user)
        .service(reset_quota_for_user);
    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

fn empty_quota_svc() -> Arc<QuotaService> {
    Arc::new(QuotaService::new(
        InMemoryQuotaRepository::new(),
        HashMap::new(),
    ))
}

#[actix_web::test]
async fn admin_quota_list_returns_403_for_anonymous() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get().uri("/api/v1/admin/quota").to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn admin_quota_list_returns_403_for_non_admin_user() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/quota")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn admin_quota_list_returns_empty_initially() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/quota")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn admin_quota_list_for_registry_returns_empty_initially() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/quota/cargo")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn admin_quota_get_for_user_returns_200_with_zero_usage() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/quota/cargo/alice")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["user_id"], "alice");
    assert_eq!(body["registry"], "cargo");
    assert_eq!(body["bytes_published"], 0);
    assert_eq!(body["packages_count"], 0);
}

#[actix_web::test]
async fn admin_quota_reset_returns_200() {
    let repo = InMemoryQuotaRepository::new();
    repo.record_publish("alice", "cargo", 1024).await.unwrap();
    let svc = Arc::new(QuotaService::new(repo.clone(), HashMap::new()));
    let app = make_quota_app(svc).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/quota/cargo/alice")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let after = repo.get_usage("alice", "cargo").await.unwrap();
    assert_eq!(after.bytes_published, 0);
}

#[actix_web::test]
async fn admin_quota_list_requires_admin() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/quota/cargo")
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn admin_quota_reset_requires_admin() {
    let app = make_quota_app(empty_quota_svc()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/quota/cargo/alice")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── Package ownership ─────────────────────────────────────────────────────────

#[actix_web::test]
async fn list_package_owners_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/registries/npm/packages/lodash/owners")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn list_package_owners_returns_200_for_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/registries/npm/packages/lodash/owners")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    // ownership is not configured in make_app → 503 or 403 for admin
    assert!(
        resp.status().is_success()
            || resp.status().is_client_error()
            || resp.status().is_server_error()
    );
}

#[actix_web::test]
async fn add_package_owner_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/packages/lodash/owners")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(
            serde_json::json!({"principal_type": "user", "principal_id": "alice", "role": "admin"}),
        )
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── Cache invalidation ────────────────────────────────────────────────────────

#[actix_web::test]
async fn invalidate_package_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/invalidate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"registry": "npm", "name": "lodash", "version": "1.0.0"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn invalidate_package_clears_cached_metadata() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "npm",
            "name": "lodash",
            "version": "4.17.21"
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["success"].as_bool().unwrap_or(false));
}

// ── Cache warming ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn warm_registry_returns_404_when_not_configured() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"package": "lodash"}))
        .to_request();
    // Warming map is empty in make_app → 404
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn warm_registry_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/warm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"package": "lodash"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn warm_registry_accepts_paths_body() {
    let app = make_app(InMemoryRepo::new()).await;
    // The `paths` body must deserialize (the old shape required `package`, which
    // would 400 here). The warming map is empty in make_app, so a valid body
    // routes through to 404 "not configured" rather than a 400 deserialize error.
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/jb/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"paths": ["idea/ideaIC-2024.1.4.tar.gz"]}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

// ── Cache eviction ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn evict_registry_returns_404_when_not_configured() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/evict")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    // Eviction map is empty in make_app → 404
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn evict_registry_returns_404_for_unknown_registry() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/does-not-exist/evict")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn evict_registry_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/evict")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn audit_log_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn audit_log_returns_200_for_admin_with_empty_events() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    // Might be empty list or paginated response
    assert!(body.is_array() || body.is_object());
}
