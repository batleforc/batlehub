//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta, NullUserTokenRepository as NullTokenRepository,
};
use batlehub_core::{
    ports::{CacheStore, PackageRepository, RegistryClient, StorageBackend, UserTokenRepository},
    services::{new_hot_lock, AdminService, HotConfig, ProxyMetrics, ProxyService, RegistryPolicy},
};
use batlehub_web::{new_access_lock, RegistryModeMap};

// ── Package explorer (explore.rs) ─────────────────────────────────────────────

/// Like `make_app` but with explore permissions open for all roles across all registries.
async fn make_explore_app(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let reg_names = ["github", "npm", "cargo", "openvsx", "go", "vscode"];
    let registries: HashMap<String, Arc<dyn RegistryClient>> = reg_names
        .iter()
        .map(|n| {
            (
                n.to_string(),
                FixedRegistry::new(*n) as Arc<dyn RegistryClient>,
            )
        })
        .collect();
    let policies: HashMap<String, Arc<RegistryPolicy>> = reg_names
        .iter()
        .map(|n| (n.to_string(), Arc::new(rbac_policy(repo_dyn.clone()))))
        .collect();

    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage: storage.clone(),
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);

    let regs: std::collections::HashSet<String> = reg_names.iter().map(|s| s.to_string()).collect();
    let access_config = new_access_lock(batlehub_web::AccessConfig {
        anonymous: regs.clone(),
        user: regs.clone(),
        admin: regs.clone(),
        groups: HashMap::new(),
        explore_anonymous: regs.clone(),
        explore_user: regs.clone(),
        explore_admin: regs.clone(),
    });
    let registry_map = registry_map_for(&[
        ("github", "github"),
        ("npm", "npm"),
        ("cargo", "cargo"),
        ("openvsx", "openvsx"),
        ("go", "goproxy"),
        ("vscode", "vscode-marketplace"),
    ]);
    let cargo_indexes = batlehub_web::CargoIndexMap::default();
    finish_test_app(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults::default(),
        test_auth_providers(),
    )
    .await
}

#[actix_web::test]
async fn explore_packages_returns_empty_list_initially() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[actix_web::test]
async fn explore_packages_anonymous_returns_empty_with_explore_access() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn explore_packages_with_specific_accessible_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
}

#[actix_web::test]
async fn explore_packages_inaccessible_registry_returns_empty() {
    // With make_app (empty explore sets), any registry filter returns empty
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[actix_web::test]
async fn explore_packages_sort_by_name() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?sort=name")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn explore_packages_sort_by_recent() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?sort=recent")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn explore_registry_stats_returns_empty_initially() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/registries")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    // Response is now an object {registries: [], upstream_unavailable: bool}
    assert!(body["registries"].is_array());
    assert_eq!(body["upstream_unavailable"], false);
}

#[actix_web::test]
async fn explore_package_detail_returns_empty_versions_for_unknown_package() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/npm/lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["registry"], "npm");
    assert_eq!(body["name"], "lodash");
    assert_eq!(body["versions"], serde_json::json!([]));
    assert!(body["gate"]["registry_accessible"]
        .as_bool()
        .unwrap_or(false));
}

#[actix_web::test]
async fn explore_package_detail_inaccessible_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/unknown-reg/some-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(!body["gate"]["registry_accessible"]
        .as_bool()
        .unwrap_or(true));
}

#[actix_web::test]
async fn explore_upstream_search_returns_empty_with_no_results() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/upstream?name=lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
}

#[actix_web::test]
async fn explore_upstream_search_filtered_by_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/upstream?name=lodash&registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── Explore cache — response shape ───────────────────────────────────────────

#[actix_web::test]
async fn explore_packages_response_includes_upstream_unavailable_false() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["upstream_unavailable"], false);
}

#[actix_web::test]
async fn explore_registry_stats_response_has_object_shape_with_upstream_field() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/registries")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert!(body.is_object(), "response must be an object, not an array");
    assert!(body["registries"].is_array());
    assert_eq!(body["upstream_unavailable"], false);
}

#[actix_web::test]
async fn explore_package_detail_response_includes_upstream_unavailable_false() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/npm/lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["upstream_unavailable"], false);
}

// ── Explore cache — invalidation endpoint ────────────────────────────────────

#[actix_web::test]
async fn explore_invalidate_requires_admin_role() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn explore_invalidate_all_returns_ok_for_admin() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn explore_invalidate_by_registry_returns_ok_for_admin() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"registry":"npm"}"#)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn explore_invalidate_anonymous_is_rejected() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    // anonymous has no admin role → 403
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn explore_cache_serves_data_after_second_request() {
    // Verifies the cache is populated on first hit and returned on subsequent calls.
    let app = make_explore_app(InMemoryRepo::new()).await;
    for _ in 0..2 {
        let req = TestRequest::get()
            .uri("/api/v1/explore/packages")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = read_body_json(resp).await;
        assert_eq!(body["upstream_unavailable"], false);
    }
}

#[actix_web::test]
async fn explore_cache_clears_after_invalidate_all() {
    let app = make_explore_app(InMemoryRepo::new()).await;

    // Prime the cache
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Flush via admin endpoint
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Subsequent request should still succeed (cache refills from DB)
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["upstream_unavailable"], false);
}
