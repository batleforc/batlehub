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
        license: Some("MIT".to_owned()),
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

// ── What the package page is told about SBOMs ────────────────────────────────
//
// The console draws a download control per format. It used to draw both
// unconditionally and discover on the click which of them existed — so an
// SPDX-only registry offered a CycloneDX download whose only possible outcome
// was a `404` behind a spinner, and every version this instance holds no bytes
// of offered two. Which formats a registry records is `[registries.sbom]`, and
// whether *this* version has any depends on whether we ever held it: both are
// facts only the server has, so the detail endpoint states them.

/// Publish `name`/`version` to the local cargo registry, so the detail endpoint
/// has a row this instance holds rather than an upstream-only candidate.
async fn publish_crate(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    name: &str,
    version: &str,
) {
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(make_publish_payload(name, version))
        .to_request();
    assert_eq!(call_service(app, req).await.status(), 200);
}

fn recorded_sbom(name: &str, version: &str, format: SbomFormat) -> ArtifactSbom {
    ArtifactSbom {
        id: Uuid::new_v4(),
        artifact_key: format!("local-cargo/{name}/{version}"),
        registry: "local-cargo".to_owned(),
        package_name: name.to_owned(),
        version: version.to_owned(),
        spec_version: format.spec_version().to_owned(),
        format,
        document: serde_json::json!({"name": name}),
        source: SbomSource::Generated,
        created_at: Utc::now(),
        license: None,
    }
}

async fn detail_of(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    name: &str,
) -> Value {
    let req = TestRequest::get()
        .uri(&format!("/api/v1/explore/packages/local-cargo/{name}"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200);
    read_body_json(resp).await
}

#[actix_web::test]
async fn explore_detail_offers_only_the_sbom_format_recorded() {
    let repo = InMemorySbomRepository::new();
    repo.upsert_sbom(recorded_sbom("my-crate", "0.1.0", SbomFormat::Spdx))
        .await
        .unwrap();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;
    publish_crate(&app, "my-crate", "0.1.0").await;

    let body = detail_of(&app, "my-crate").await;

    assert_eq!(body["versions"][0]["sbom"]["state"], "available");
    assert_eq!(
        body["versions"][0]["sbom"]["formats"],
        serde_json::json!(["spdx"]),
        "a CycloneDX button here would be a link that can only 404"
    );
}

/// Ordered by the format, not by whatever the store returned, so the two
/// controls do not swap places between rows.
#[actix_web::test]
async fn explore_detail_lists_both_sbom_formats_when_both_were_recorded() {
    let repo = InMemorySbomRepository::new();
    for format in [SbomFormat::CycloneDx, SbomFormat::Spdx] {
        repo.upsert_sbom(recorded_sbom("my-crate", "0.1.0", format))
            .await
            .unwrap();
    }
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;
    publish_crate(&app, "my-crate", "0.1.0").await;

    let body = detail_of(&app, "my-crate").await;

    assert_eq!(
        body["versions"][0]["sbom"]["formats"],
        serde_json::json!(["spdx", "cyclonedx"])
    );
}

/// A version we hold with nothing recorded is a definite `none` — the console
/// says so instead of offering a download.
#[actix_web::test]
async fn explore_detail_says_none_for_a_held_version_with_no_sbom() {
    let sbom_svc = Arc::new(SbomService::new(InMemorySbomRepository::new(), None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;
    publish_crate(&app, "my-crate", "0.1.0").await;

    let body = detail_of(&app, "my-crate").await;

    assert_eq!(body["versions"][0]["sbom"]["state"], "none");
    assert_eq!(
        body["versions"][0]["sbom"]["formats"],
        serde_json::json!([])
    );
}

/// An SBOM recorded for *another* version is not this version's.
#[actix_web::test]
async fn explore_detail_does_not_borrow_another_versions_sbom() {
    let repo = InMemorySbomRepository::new();
    repo.upsert_sbom(recorded_sbom("my-crate", "0.1.0", SbomFormat::Spdx))
        .await
        .unwrap();
    let sbom_svc = Arc::new(SbomService::new(repo, None, None));
    let app = make_local_registry_app_with_sbom(RegistryMode::Local, Some(sbom_svc)).await;
    publish_crate(&app, "my-crate", "0.2.0").await;

    let body = detail_of(&app, "my-crate").await;

    let row = body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["version"] == "0.2.0")
        .expect("the published version is on the page");
    assert_eq!(row["sbom"]["state"], "none");
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
