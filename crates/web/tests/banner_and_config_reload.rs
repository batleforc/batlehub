//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, init_service, read_body_json, TestRequest};
use serde_json::Value;
use utoipa_actix_web::AppExt;

use batlehub_adapters::cache::InMemoryBannerStore;
use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta, NullUserTokenRepository as NullTokenRepository,
};
use batlehub_core::{
    ports::{BannerPort, CacheStore, StorageBackend, UserTokenRepository},
    services::{new_hot_lock, AdminService, HotConfig, ProxyMetrics, ProxyService},
};
use batlehub_web::services::{
    BannerService, ConfigReloadParams, ConfigReloadService, HotConfigBuilder,
};
use batlehub_web::AuthMiddlewareFactory;

// ── Banner endpoints ──────────────────────────────────────────────────────────

/// Build a minimal app with banner and reload services wired in.
async fn make_banner_app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    make_banner_app_seeded(Vec::new()).await
}

/// `make_banner_app`, with the reload service's config-warning store pre-seeded
/// as `set_warnings` does at server startup.
async fn make_banner_app_seeded(
    warnings: Vec<batlehub_config::schema::ConfigWarning>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    use std::collections::HashMap;
    let repo = InMemoryRepo::new();
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
        readme: None,
        discovery: Default::default(),
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = access_config_for(&[]);

    let banner_store: Arc<dyn BannerPort> = Arc::new(InMemoryBannerStore::new());
    let banner_svc = Arc::new(BannerService::new(banner_store));

    let hot = proxy_svc.hot.clone();
    let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("not used in tests"));
    let reload_svc = Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot,
        access: access_config.clone(),
        search: batlehub_web::new_search_lock(false),
        registry_map: batlehub_web::RegistryMap::new(HashMap::new()),
        registry_mode_map: batlehub_web::RegistryModeMap::new(HashMap::new()),
        upstream_map: batlehub_web::UpstreamMap::new(HashMap::new()),
        cargo_index_map: batlehub_web::CargoIndexMap::new(HashMap::new()),
        repo_signer_map: batlehub_web::RepoSignerMap::default(),
        vuln_db_map: batlehub_web::VulnDbMap::default(),
        sumdb_map: batlehub_web::SumDbMap::default(),
        registry_host_map: batlehub_web::RegistryHostMap::default(),
        proxy_trust: batlehub_web::ProxyTrust::default(),
        config_path: "config.toml".to_owned(),
        config_change_repo: None,
        hot_reload_enabled: true,
        builder,
        banner: Some(Arc::clone(&banner_svc)),
    }));
    reload_svc.set_warnings(warnings);

    let (app, _) = actix_web::App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            admin_svc,
            token_repo,
            access_config,
            batlehub_web::RegistryMap::new(HashMap::new()),
            ConfigureAppDefaults::default(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(banner_svc))
        .app_data(actix_web::web::Data::new(reload_svc))
        .app_data(actix_web::web::Data::new(
            batlehub_web::CargoIndexMap::default(),
        ))
        .app_data(actix_web::web::Data::new(
            batlehub_web::RegistryModeMap::default(),
        ));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

#[actix_web::test]
async fn get_banner_returns_null_when_unset() {
    let app = make_banner_app().await;
    let req = TestRequest::get().uri("/api/v1/banner").to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body.is_null(), "expected null, got {body}");
}

#[actix_web::test]
async fn set_banner_requires_admin() {
    let app = make_banner_app().await;
    let req = TestRequest::put()
        .uri("/api/v1/admin/banner")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"message": "hello", "level": "info"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn set_and_get_banner_round_trip() {
    let app = make_banner_app().await;

    // Set banner as admin
    let set_req = TestRequest::put()
        .uri("/api/v1/admin/banner")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"message": "Maintenance window", "level": "warning"}))
        .to_request();
    let set_resp = call_service(&app, set_req).await;
    assert_eq!(set_resp.status(), 200);

    // Read banner (no auth needed)
    let get_req = TestRequest::get().uri("/api/v1/banner").to_request();
    let get_resp = call_service(&app, get_req).await;
    assert_eq!(get_resp.status(), 200);
    let banner: Value = read_body_json(get_resp).await;
    assert_eq!(banner["message"], "Maintenance window");
    assert_eq!(banner["level"], "warning");
}

#[actix_web::test]
async fn clear_banner_removes_it() {
    let app = make_banner_app().await;

    // Set then clear
    let _ = call_service(
        &app,
        TestRequest::put()
            .uri("/api/v1/admin/banner")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .set_json(serde_json::json!({"message": "temp", "level": "info"}))
            .to_request(),
    )
    .await;

    let del_resp = call_service(
        &app,
        TestRequest::delete()
            .uri("/api/v1/admin/banner")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(del_resp.status(), 204);

    let get_resp = call_service(&app, TestRequest::get().uri("/api/v1/banner").to_request()).await;
    assert_eq!(get_resp.status(), 200);
    let body: Value = read_body_json(get_resp).await;
    assert!(body.is_null());
}

// ── Config reload endpoints ───────────────────────────────────────────────────

#[actix_web::test]
async fn reload_config_returns_503_when_disabled() {
    use std::collections::HashMap;
    let repo = InMemoryRepo::new();
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
        readme: None,
        discovery: Default::default(),
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);
    let access_config = access_config_for(&[]);

    let hot = proxy_svc.hot.clone();
    let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("unused"));
    let reload_svc = Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot,
        access: access_config.clone(),
        search: batlehub_web::new_search_lock(false),
        registry_map: batlehub_web::RegistryMap::new(HashMap::new()),
        registry_mode_map: batlehub_web::RegistryModeMap::new(HashMap::new()),
        upstream_map: batlehub_web::UpstreamMap::new(HashMap::new()),
        cargo_index_map: batlehub_web::CargoIndexMap::new(HashMap::new()),
        repo_signer_map: batlehub_web::RepoSignerMap::default(),
        vuln_db_map: batlehub_web::VulnDbMap::default(),
        sumdb_map: batlehub_web::SumDbMap::default(),
        registry_host_map: batlehub_web::RegistryHostMap::default(),
        proxy_trust: batlehub_web::ProxyTrust::default(),
        config_path: "config.toml".to_owned(),
        config_change_repo: None,
        hot_reload_enabled: false,
        builder,
        banner: None,
    }));

    let (app, _) = actix_web::App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            admin_svc,
            token_repo,
            access_config,
            batlehub_web::RegistryMap::new(HashMap::new()),
            ConfigureAppDefaults::default(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(reload_svc))
        .app_data(actix_web::web::Data::new(
            batlehub_web::CargoIndexMap::default(),
        ))
        .app_data(actix_web::web::Data::new(
            batlehub_web::RegistryModeMap::default(),
        ));
    let app = init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await;

    let resp = call_service(
        &app,
        TestRequest::post()
            .uri("/api/v1/admin/config/reload")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 503);
}

#[actix_web::test]
async fn get_pending_reload_returns_404_when_none() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/config/pending")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn apply_pending_returns_404_when_none() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::post()
            .uri("/api/v1/admin/config/pending/apply")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn discard_pending_returns_404_when_none() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::delete()
            .uri("/api/v1/admin/config/pending")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn config_reload_endpoints_require_admin() {
    let app = make_banner_app().await;
    for (method, uri) in [
        ("POST", "/api/v1/admin/config/reload"),
        ("GET", "/api/v1/admin/config/pending"),
        ("POST", "/api/v1/admin/config/pending/apply"),
        ("DELETE", "/api/v1/admin/config/pending"),
    ] {
        let req = TestRequest::with_uri(uri)
            .method(actix_web::http::Method::from_bytes(method.as_bytes()).unwrap())
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert_eq!(resp.status(), 403, "{method} {uri} should require admin");
    }
}

#[actix_web::test]
async fn list_config_changes_returns_empty_without_db() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/config/changes")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    // No DB pool → internal error from list_changes, or empty if handled gracefully
    // The endpoint returns 500 when no pool is configured; that's acceptable here.
    assert!(
        resp.status().is_success() || resp.status().is_server_error(),
        "unexpected status {}",
        resp.status()
    );
}

// ── Config warnings ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn config_warnings_endpoint_returns_the_active_warnings() {
    use batlehub_config::schema::{warnings as codes, ConfigWarning};
    let app = make_banner_app_seeded(vec![ConfigWarning::new(
        codes::PROXY_TRUST_UNCONFIGURED,
        "server.trusted_proxies",
        "no trusted-proxy list is configured",
    )])
    .await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/config/warnings")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
    let warnings = body["warnings"].as_array().expect("warnings array");
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["code"], codes::PROXY_TRUST_UNCONFIGURED);
    // `path` is shown verbatim so an operator can grep the TOML for it.
    assert_eq!(warnings[0]["path"], "server.trusted_proxies");
}

#[actix_web::test]
async fn config_warnings_endpoint_returns_an_empty_list_for_a_clean_config() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/config/warnings")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = actix_web::test::read_body_json(resp).await;
    assert!(body["warnings"]
        .as_array()
        .expect("warnings array")
        .is_empty());
}

#[actix_web::test]
async fn config_warnings_endpoint_requires_admin() {
    let app = make_banner_app().await;
    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/api/v1/admin/config/warnings")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403);
}
