//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use chrono::Utc;
use serde_json::Value;

use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemorySbomRepository,
};
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{ArtifactSbom, SbomFormat, SbomSource},
    ports::SbomRepository,
    services::SbomService,
};
use uuid::Uuid;

// ── SBOM endpoints ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn get_artifact_sbom_anonymous_returns_403() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/local-cargo/my-crate/1.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn get_artifact_sbom_unknown_format_returns_400() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/local-cargo/my-crate/1.0.0?format=bogus")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn get_artifact_sbom_not_found_returns_404() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/local-cargo/my-crate/1.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn get_artifact_sbom_returns_document() {
    let repo = InMemorySbomRepository::new();
    repo.upsert_sbom(ArtifactSbom {
        id: Uuid::new_v4(),
        artifact_key: "cargo/my-crate/1.0.0".to_owned(),
        registry: "local-cargo".to_owned(),
        package_name: "my-crate".to_owned(),
        version: "1.0.0".to_owned(),
        format: SbomFormat::Spdx,
        spec_version: "SPDX-2.3".to_owned(),
        document: serde_json::json!({"spdxVersion": "SPDX-2.3", "name": "my-crate"}),
        source: SbomSource::Generated,
        created_at: Utc::now(),
    })
    .await
    .unwrap();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/local-cargo/my-crate/1.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/json"
    );
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["spdxVersion"], "SPDX-2.3");
}

#[actix_web::test]
async fn export_org_sbom_non_admin_returns_403() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/export")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn export_org_sbom_unknown_format_returns_400() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/export?format=bogus")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn export_org_sbom_returns_attachment() {
    let repo = InMemorySbomRepository::new();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;

    let req = TestRequest::get()
        .uri("/api/v1/sbom/export")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let disposition = resp
        .headers()
        .get("Content-Disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.starts_with("attachment; filename=\"sbom-export-all-"));
    assert!(disposition.ends_with("spdx.json\""));
}

// ── Concurrent load smoke test ───────────────────────────────────────────────
//
// Stands in for the k6/Podman perf harness (`perf/k6`, `task perf:run:*`) in
// environments where Podman isn't available. Fires many concurrent requests
// through `ProxyService::handle` against the in-memory backends, mixing
// metadata reads (cache hit/miss) and authenticated tarball downloads
// (artifact cache hit/miss) across a handful of distinct packages. It is a
// correctness regression net for the hot-path changes (no panics, no
// deadlocks, every response succeeds) — not a substitute for real RSS/CPU
// numbers, which still require `task perf:run:mixed` against real Postgres.

#[actix_web::test]
async fn proxy_handles_concurrent_mixed_requests_without_errors() {
    let app = make_app(InMemoryRepo::new()).await;

    // 8 distinct packages, requested repeatedly: each package's first hit is a
    // metadata/artifact cache miss, every subsequent one a cache hit.
    let packages: Vec<String> = (0..8).map(|i| format!("pkg-{i}")).collect();

    let requests: Vec<_> = (0..200)
        .map(|i| {
            let pkg = &packages[i % packages.len()];
            if i % 2 == 0 {
                TestRequest::get()
                    .uri(&format!("/proxy/npm/{pkg}"))
                    .to_request()
            } else {
                TestRequest::get()
                    .uri(&format!("/proxy/npm/{pkg}/1.0.0/tarball"))
                    .insert_header(("Authorization", bearer(USER_TOKEN)))
                    .to_request()
            }
        })
        .collect();

    let responses =
        futures::future::join_all(requests.into_iter().map(|req| call_service(&app, req))).await;

    for resp in responses {
        assert_eq!(
            resp.status(),
            200,
            "every concurrent proxy request should succeed"
        );
    }
}
