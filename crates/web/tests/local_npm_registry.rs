//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use base64::Engine as _;
use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_config::schema::RegistryMode;

// ── Local / Hybrid private npm registry ───────────────────────────────────────

/// Build a test app with a single npm registry in the given mode.
/// Registry name is `"local-npm"`, type `"npm"`.
async fn make_local_npm_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-npm", "npm", mode, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

/// Build a standard npm publish payload (the wire format used by `npm publish`).
fn make_npm_publish_payload(name: &str, version: &str) -> serde_json::Value {
    let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"fake-tarball-content");
    serde_json::json!({
        "name": name,
        "versions": {
            version: {
                "name": name,
                "version": version,
                "description": "Test package",
                "dist": {
                    "shasum": "abc123"
                }
            }
        },
        "_attachments": {
            format!("{}-{}.tgz", name, version): {
                "content_type": "application/octet-stream",
                "data": tarball_b64,
                "length": 20
            }
        }
    })
}

#[actix_web::test]
async fn npm_publish_user_can_publish() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-package")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn npm_publish_duplicate_version_returns_409() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dup-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dup-pkg", "1.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dup-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dup-pkg", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn npm_publish_anonymous_returns_403() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-package")
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn npm_publish_proxy_mode_returns_404() {
    // `npm` registry in make_app uses mode=Proxy (default)
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::put()
        .uri("/proxy/npm/my-package")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-package", "1.0.0"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn npm_publish_traversal_version_returns_400() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    // A traversal sequence in the version field must be rejected before reaching
    // the storage layer so that `validate_path_safe` returns a clean 400.
    let req = TestRequest::put()
        .uri("/proxy/local-npm/legit-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("legit-pkg", "../../etc/x"))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn npm_publish_traversal_name_returns_400() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    // `evil%2F..` URL-decodes to `evil/..` which contains a `..` path segment —
    // rejected by validate_package_name before reaching the storage layer.
    let req = TestRequest::put()
        .uri("/proxy/local-npm/evil%2F..")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("evil/..", "1.0.0"))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn npm_packument_returns_published_version() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/my-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("my-pkg", "2.0.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/my-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["name"], "my-pkg");
    assert!(
        body["versions"]["2.0.0"].is_object(),
        "published version must appear in packument"
    );
    assert!(
        body["versions"]["2.0.0"]["dist"]["tarball"]
            .as_str()
            .unwrap_or("")
            .contains("/proxy/local-npm/my-pkg/2.0.0/tarball"),
        "tarball URL must be rewritten to BatleHub serving path"
    );
}

#[actix_web::test]
async fn npm_version_returns_metadata() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/ver-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("ver-pkg", "0.5.0"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/ver-pkg/0.5.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["version"], "0.5.0");
    assert!(
        body["dist"]["tarball"]
            .as_str()
            .unwrap_or("")
            .contains("/proxy/local-npm/ver-pkg/0.5.0/tarball"),
        "tarball URL must point at BatleHub"
    );
}

#[actix_web::test]
async fn npm_tarball_download_returns_artifact() {
    let app = make_local_npm_app(RegistryMode::Local).await;

    let req = TestRequest::put()
        .uri("/proxy/local-npm/dl-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(make_npm_publish_payload("dl-pkg", "1.2.3"))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-npm/dl-pkg/1.2.3/tarball")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), b"fake-tarball-content");
}

#[actix_web::test]
async fn npm_tarball_unknown_version_returns_404() {
    let app = make_local_npm_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-npm/no-pkg/9.9.9/tarball")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}
