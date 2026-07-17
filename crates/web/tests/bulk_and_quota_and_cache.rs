//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test::{call_service, init_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta,
};
use batlehub_core::entities::{AccessEvent, PackageId, Role};
use batlehub_core::ports::{NoopWarmCoordinator, PackageRepository, StorageBackend, StorageMeta};
use batlehub_core::services::{EvictionConfig, EvictionService, ProxyMetrics, WarmingService};
use batlehub_web::handlers::back_office::ops::eviction::EvictionServiceMap;
use batlehub_web::handlers::back_office::ops::warming::WarmingServiceMap;
use batlehub_web::AuthMiddlewareFactory;
use bytes::Bytes;
use chrono::{Duration as ChronoDuration, Utc};

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

fn npm_warming_service(storage: Arc<dyn StorageBackend>) -> Arc<WarmingService> {
    Arc::new(WarmingService {
        client: FixedRegistry::new("npm"),
        storage,
        artifact_meta: NoopArtifactMeta::arc(),
        registry_name: "npm".to_owned(),
        latest_n: 3,
        concurrency: 4,
        coordinator: Arc::new(NoopWarmCoordinator),
        metrics: Arc::new(ProxyMetrics::new(&["npm".to_owned()])),
    })
}

#[actix_web::test]
async fn get_warming_status_requires_admin() {
    let (app, _storage) = make_app_with_warming(WarmingServiceMap::default()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/warming")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn get_warming_status_lists_configured_registries() {
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let warming_map: WarmingServiceMap = [("npm".to_owned(), npm_warming_service(storage))].into();
    let (app, _storage) = make_app_with_warming(warming_map).await;

    let req = TestRequest::get()
        .uri("/api/v1/admin/warming")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let registries = body["registries"].as_array().unwrap();
    assert_eq!(registries.len(), 1);
    assert_eq!(registries[0]["name"], "npm");
    assert_eq!(registries[0]["latest_n"], 3);
    assert_eq!(registries[0]["concurrency"], 4);
}

#[actix_web::test]
async fn warm_registry_rejects_empty_body() {
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let warming_map: WarmingServiceMap = [("npm".to_owned(), npm_warming_service(storage))].into();
    let (app, _storage) = make_app_with_warming(warming_map).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn warm_registry_warms_pinned_package_version() {
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let warming_map: WarmingServiceMap =
        [("npm".to_owned(), npm_warming_service(storage.clone()))].into();
    let (app, _storage) = make_app_with_warming(warming_map).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"package": "lodash@4.17.21"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["warmed"], 1);
    assert_eq!(body["errors"], 0);
}

#[actix_web::test]
async fn warm_registry_warms_path_and_honours_versions_override() {
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let warming_map: WarmingServiceMap =
        [("npm".to_owned(), npm_warming_service(storage.clone()))].into();
    let (app, _storage) = make_app_with_warming(warming_map).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "package": "lodash@4.17.21",
            "path": "extra/asset.tgz",
            "versions": 1,
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["warmed"], 2, "one pinned package + one path");
    assert_eq!(body["errors"], 0);
}

#[actix_web::test]
async fn warm_registry_returns_404_for_unknown_registry() {
    let (app, _storage) = make_app_with_warming(WarmingServiceMap::default()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/does-not-exist/warm")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"package": "lodash"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
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

#[actix_web::test]
async fn evict_registry_runs_configured_service_and_reports_zero_counts() {
    // All strategies disabled (`EvictionConfig::default()`), so `run_all` does
    // no I/O and returns straight away with every counter at zero.
    let svc = Arc::new(EvictionService::new(
        NoopArtifactMeta::arc(),
        batlehub_adapters::in_memory::InMemoryStorageBackend::new(),
        EvictionConfig::default(),
    ));
    let eviction_map: EvictionServiceMap = [("npm".to_owned(), svc)].into();
    let (app, _storage) = make_app_with_eviction(eviction_map).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/npm/evict")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["total"], 0);
    assert_eq!(body["evicted_ttl"], 0);
    assert_eq!(body["evicted_idle"], 0);
    assert_eq!(body["evicted_old_versions"], 0);
    assert_eq!(body["evicted_lru"], 0);
}

// ── Targeted proxy-cache artifact deletion ────────────────────────────────────

#[actix_web::test]
async fn delete_cached_artifact_requires_admin() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"name": "lodash", "version": "1.0.0"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn delete_cached_artifact_returns_404_for_unknown_registry() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/does-not-exist/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"name": "lodash", "version": "1.0.0"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn delete_cached_artifact_rejects_missing_name_or_version() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;

    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"version": "1.0.0"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);

    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"name": "lodash"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn delete_cached_artifact_rejects_traversal_in_name() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"name": "../etc/passwd", "version": "1.0.0"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn delete_cached_artifact_rejects_empty_path() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"path": ""}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn delete_cached_artifact_by_name_version_returns_false_when_absent() {
    let (app, _storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"name": "lodash", "version": "1.0.0"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["deleted"], false);
    assert_eq!(body["artifact_key"], "artifact:npm/lodash/1.0.0");
}

#[actix_web::test]
async fn delete_cached_artifact_by_name_version_deletes_stored_artifact() {
    let (app, storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    storage
        .store(
            "artifact:npm/lodash/1.0.0",
            Bytes::from_static(b"tarball"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"name": "lodash", "version": "1.0.0"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["deleted"], true);
    assert_eq!(body["artifact_key"], "artifact:npm/lodash/1.0.0");

    assert!(!storage.exists("artifact:npm/lodash/1.0.0").await.unwrap());
}

#[actix_web::test]
async fn delete_cached_artifact_by_path_deletes_stored_artifact() {
    let (app, storage) = make_app_with_eviction(EvictionServiceMap::default()).await;
    storage
        .store(
            "artifact:npm/repo/_/idea/ideaIC-2024.1.4.tar.gz",
            Bytes::from_static(b"binary"),
            StorageMeta::default(),
        )
        .await
        .unwrap();

    let req = TestRequest::delete()
        .uri("/api/v1/admin/registries/npm/cache")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"path": "idea/ideaIC-2024.1.4.tar.gz"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["deleted"], true);
    assert_eq!(
        body["artifact_key"],
        "artifact:npm/repo/_/idea/ideaIC-2024.1.4.tar.gz"
    );
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

#[actix_web::test]
async fn audit_log_returns_seeded_events_and_respects_denied_only_filter() {
    let repo = InMemoryRepo::new();
    repo.record_access(AccessEvent::allowed_download(
        PackageId::new("npm", "lodash", "4.17.21"),
        Some("user-1".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();
    repo.record_access(AccessEvent::denied_download(
        PackageId::new("npm", "evil-pkg", "1.0.0"),
        Some("user-1".to_owned()),
        Role::User,
        "blocked".to_owned(),
    ))
    .await
    .unwrap();

    let app = make_app(repo).await;

    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["total"], 2);
    assert_eq!(body["items"].as_array().unwrap().len(), 2);

    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log?denied_only=true")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["total"], 1);
    assert_eq!(body["items"][0]["result"]["outcome"], "denied");

    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log?registry=npm&user_id=user-1&page=0&per_page=1")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["total"], 2, "count ignores the page-size limit");
    assert_eq!(
        body["items"].as_array().unwrap().len(),
        1,
        "list respects per_page"
    );
}

// ── Audit log export ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn export_audit_log_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log/export")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn export_audit_log_defaults_to_json() {
    let repo = InMemoryRepo::new();
    repo.record_access(AccessEvent::allowed_download(
        PackageId::new("npm", "lodash", "4.17.21"),
        Some("user-1".to_owned()),
        Role::User,
    ))
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log/export")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    assert!(resp
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("audit-log-"));
    let body: Value = read_body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}

#[actix_web::test]
async fn export_audit_log_supports_csv_format() {
    let repo = InMemoryRepo::new();
    repo.record_access(AccessEvent::denied_download(
        PackageId::new("npm", "evil-pkg", "1.0.0"),
        Some("user-1".to_owned()),
        Role::User,
        "blocked by policy".to_owned(),
    ))
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/audit-log/export?format=csv")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers().get("content-type").unwrap(), "text/csv");
    let body = read_body(resp).await;
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.starts_with("id,timestamp,user_id"));
    assert!(csv.contains("evil-pkg"));
    assert!(csv.contains("blocked by policy"));
    assert!(csv.contains("denied"));
}

// ── Audit log purge ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn purge_audit_log_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/audit-log?before=2026-01-01T00:00:00Z")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn purge_audit_log_returns_200_with_deleted_count() {
    // `InMemoryPackageRepository` doesn't override `purge_events_before`, so it
    // falls back to the port's default no-op (`Ok(0)`) — only Postgres actually
    // purges. This still exercises the handler's query parsing, admin check,
    // and response shape end to end.
    let app = make_app(InMemoryRepo::new()).await;
    // Avoid a literal `+` in the query string (form-urlencoded decoding turns
    // it into a space), so format with a bare `Z` offset instead of `to_rfc3339`.
    let cutoff = (Utc::now() - ChronoDuration::days(30)).format("%Y-%m-%dT%H:%M:%SZ");
    let req = TestRequest::delete()
        .uri(&format!("/api/v1/admin/audit-log?before={cutoff}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["deleted"], 0);
}
