//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test::{call_service, init_service, read_body_json, TestRequest};
use actix_web::App;
use serde_json::Value;
use utoipa_actix_web::AppExt;

use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta, NullUserTokenRepository as NullTokenRepository,
};
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    ports::{CacheStore, RegistryClient, StorageBackend},
    services::{new_hot_lock, AdminService, HotConfig, ProxyMetrics, ProxyService},
};
use batlehub_web::{AuthMiddlewareFactory, RegistryModeMap, RepoSignerMap};

// ─────────────────────────────────────────────────────────────────────────────
// ── Vulnerability proxy endpoints ────────────────────────────────────────────
// ─────────────────────────────────────────────────────────────────────────────

// ── npm audit bulk ────────────────────────────────────────────────────────────

#[actix_web::test]
async fn audit_bulk_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/cargo/-/npm/v1/audit/bulk")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn audit_bulk_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/no-such/-/npm/v1/audit/bulk")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn audit_bulk_no_upstream_configured_returns_404() {
    // make_app uses UpstreamMap::default() (empty) → no upstream for "npm" → 404
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/proxy/npm/-/npm/v1/audit/bulk")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn audit_bulk_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"{}";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let upstream_url = format!("http://127.0.0.1:{port}");
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "npm".to_owned(),
        FixedRegistry::new("npm") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        [("npm".to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let mut upstream_entries = std::collections::HashMap::new();
    upstream_entries.insert("npm".to_owned(), upstream_url);
    let upstream_map = batlehub_web::UpstreamMap::from(upstream_entries);
    let app = finish_test_app(
        proxy_svc,
        Arc::new(AdminService::new(repo_dyn)),
        Arc::new(NullTokenRepository),
        access_config_for(&["npm"]),
        registry_map_for(&[("npm", "npm")]),
        local_svc,
        RegistryModeMap::default(),
        batlehub_web::CargoIndexMap::default(),
        ConfigureAppDefaults {
            upstream_map,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await;

    let req = TestRequest::post()
        .uri("/proxy/npm/-/npm/v1/audit/bulk")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"packages": {}}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

// ── NuGet vulnerability endpoints ─────────────────────────────────────────────

#[actix_web::test]
async fn nuget_vuln_index_wrong_registry_type_returns_404() {
    // "npm" exists but is not a nuget registry.
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/nuget/v3/vulnerabilities/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn nuget_vuln_index_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such/nuget/v3/vulnerabilities/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn nuget_vuln_page_wrong_registry_type_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/nuget/v3/vulnerabilities/page/0.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn nuget_vuln_page_invalid_id_returns_400() {
    // Validation runs before the upstream call; a space in the page ID is rejected.
    let app = make_local_nuget_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/vulnerabilities/page/bad%20page")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

/// Builds a minimal app with one "nuget" registry and an upstream map pointing to `upstream_url`.
async fn build_nuget_vuln_test_app(
    upstream_url: String,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "nuget".to_owned(),
        FixedRegistry::new("nuget") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        [("nuget".to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let mut upstream_entries = std::collections::HashMap::new();
    upstream_entries.insert("nuget".to_owned(), upstream_url);
    let upstream_map = batlehub_web::UpstreamMap::from(upstream_entries);
    finish_test_app(
        proxy_svc,
        Arc::new(AdminService::new(repo_dyn)),
        Arc::new(NullTokenRepository),
        access_config_for(&["nuget"]),
        registry_map_for(&[("nuget", "nuget")]),
        local_svc,
        RegistryModeMap::default(),
        batlehub_web::CargoIndexMap::default(),
        ConfigureAppDefaults {
            upstream_map,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await
}

#[actix_web::test]
async fn nuget_vuln_index_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"[]";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let app = build_nuget_vuln_test_app(format!("http://127.0.0.1:{port}")).await;
    let req = TestRequest::get()
        .uri("/proxy/nuget/nuget/v3/vulnerabilities/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn nuget_vuln_page_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"[]";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let app = build_nuget_vuln_test_app(format!("http://127.0.0.1:{port}")).await;
    let req = TestRequest::get()
        .uri("/proxy/nuget/nuget/v3/vulnerabilities/page/0.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

// ── Composer security advisories ─────────────────────────────────────────────

#[actix_web::test]
async fn composer_security_advisories_wrong_registry_type_returns_404() {
    // "npm" exists but is not a composer registry.
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/api/security-advisories/")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn composer_security_advisories_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such/api/security-advisories/")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn composer_security_advisories_no_upstream_returns_404() {
    // make_local_composer_app uses UpstreamMap::default() (empty) → handler returns 404.
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/api/security-advisories/")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn composer_security_advisories_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"{\"advisories\":{}}";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let upstream_url = format!("http://127.0.0.1:{port}");
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "packagist".to_owned(),
        FixedRegistry::new("composer") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> = [(
        "packagist".to_owned(),
        Arc::new(rbac_policy(repo_dyn.clone())),
    )]
    .into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let mode_map = RegistryModeMap::default();
    mode_map.insert("packagist".to_owned(), RegistryMode::Proxy);
    let mut upstream_entries = std::collections::HashMap::new();
    upstream_entries.insert("packagist".to_owned(), upstream_url);
    let upstream_map = batlehub_web::UpstreamMap::from(upstream_entries);
    let app = finish_test_app(
        proxy_svc,
        Arc::new(AdminService::new(repo_dyn)),
        Arc::new(NullTokenRepository),
        access_config_for(&["packagist"]),
        registry_map_for(&[("packagist", "composer")]),
        local_svc,
        mode_map,
        batlehub_web::CargoIndexMap::default(),
        ConfigureAppDefaults {
            upstream_map,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await;

    let req = TestRequest::get()
        .uri("/proxy/packagist/api/security-advisories/?packages[]=vendor/pkg")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

// ── goproxy vulnerability database ────────────────────────────────────────────

#[actix_web::test]
async fn goproxy_vuln_index_wrong_registry_type_returns_404() {
    // "npm" exists but is not a goproxy registry.
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/v1/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn goproxy_vuln_index_unknown_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such/v1/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn goproxy_vuln_entry_invalid_id_returns_400() {
    // Validation rejects IDs with spaces before the upstream is contacted.
    // "go" is a goproxy registry in make_app, so require_registry_type passes.
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/go/v1/ID/bad%20id.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

/// Builds a minimal goproxy app with a custom `VulnDbMap` (not the shared default).
async fn build_goproxy_vuln_test_app(
    vuln_db_map: batlehub_web::VulnDbMap,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "go".to_owned(),
        FixedRegistry::new("goproxy") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        [("go".to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            Arc::new(AdminService::new(repo_dyn)),
            Arc::new(NullTokenRepository),
            access_config_for(&["go"]),
            registry_map_for(&[("go", "goproxy")]),
            ConfigureAppDefaults::default(),
        ))
        .split_for_parts();

    let app = app
        .app_data(actix_web::web::Data::new(
            batlehub_web::CargoIndexMap::default(),
        ))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()))
        .app_data(actix_web::web::Data::new(RepoSignerMap::default()))
        .app_data(actix_web::web::Data::new(vuln_db_map));

    init_service(app.wrap(AuthMiddlewareFactory::new(test_auth_providers()))).await
}

#[actix_web::test]
async fn goproxy_vuln_index_disabled_returns_404() {
    // An empty string URL disables the vuln DB proxy for the registry.
    let vuln_db =
        batlehub_web::VulnDbMap::new([("go".to_owned(), String::new())].into_iter().collect());
    let app = build_goproxy_vuln_test_app(vuln_db).await;
    let req = TestRequest::get()
        .uri("/proxy/go/v1/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn goproxy_vuln_index_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"[]";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let vuln_db = batlehub_web::VulnDbMap::new(
        [("go".to_owned(), format!("http://127.0.0.1:{port}"))]
            .into_iter()
            .collect(),
    );
    let app = build_goproxy_vuln_test_app(vuln_db).await;
    let req = TestRequest::get()
        .uri("/proxy/go/v1/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn goproxy_vuln_query_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"{\"vulns\":[]}";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let vuln_db = batlehub_web::VulnDbMap::new(
        [("go".to_owned(), format!("http://127.0.0.1:{port}"))]
            .into_iter()
            .collect(),
    );
    let app = build_goproxy_vuln_test_app(vuln_db).await;
    let req = TestRequest::post()
        .uri("/proxy/go/v1/query")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"queries": []}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn goproxy_vuln_entry_forwards_to_upstream_and_returns_response() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let body = b"{\"id\":\"GO-2023-1234\"}";
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let vuln_db = batlehub_web::VulnDbMap::new(
        [("go".to_owned(), format!("http://127.0.0.1:{port}"))]
            .into_iter()
            .collect(),
    );
    let app = build_goproxy_vuln_test_app(vuln_db).await;
    let req = TestRequest::get()
        .uri("/proxy/go/v1/ID/GO-2023-1234.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["id"], "GO-2023-1234");
}

#[actix_web::test]
async fn audit_bulk_relays_upstream_response_body() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        // Upstream returns a non-trivial audit report.
        let body =
            br#"{"advisories":{"lodash":{"findings":[{"version":"4.17.15"}],"severity":"high"}}}"#;
        let _ = stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .await;
        let _ = stream.write_all(body).await;
    });

    let upstream_url = format!("http://127.0.0.1:{port}");
    let repo_dyn: Arc<dyn batlehub_core::ports::PackageRepository> = InMemoryRepo::new();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());
    let registries: HashMap<String, Arc<dyn RegistryClient>> = [(
        "npm".to_owned(),
        FixedRegistry::new("npm") as Arc<dyn RegistryClient>,
    )]
    .into();
    let policies: HashMap<String, Arc<batlehub_core::services::RegistryPolicy>> =
        [("npm".to_owned(), Arc::new(rbac_policy(repo_dyn.clone())))].into();
    let local_svc = make_local_svc(storage.clone());
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            ..Default::default()
        }),
        storage,
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
    });
    let mut upstream_entries = std::collections::HashMap::new();
    upstream_entries.insert("npm".to_owned(), upstream_url);
    let upstream_map = batlehub_web::UpstreamMap::from(upstream_entries);
    let app = finish_test_app(
        proxy_svc,
        Arc::new(AdminService::new(repo_dyn)),
        Arc::new(NullTokenRepository),
        access_config_for(&["npm"]),
        registry_map_for(&[("npm", "npm")]),
        local_svc,
        RegistryModeMap::default(),
        batlehub_web::CargoIndexMap::default(),
        ConfigureAppDefaults {
            upstream_map,
            ..Default::default()
        },
        test_auth_providers(),
    )
    .await;

    let req = TestRequest::post()
        .uri("/proxy/npm/-/npm/v1/audit/bulk")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"packages": {"lodash": ["4.17.15"]}}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    // Verify the upstream body is relayed verbatim.
    assert_eq!(body["advisories"]["lodash"]["severity"], "high");
}
