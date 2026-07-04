//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_config::schema::RegistryMode;

// ── Local / Hybrid private VS Code extension (openvsx) registry ───────────────

/// Build a test app with a single openvsx registry in the given mode.
/// Registry name is `"local-vsx"`, type `"openvsx"`.
async fn make_local_vsx_app(
    mode: RegistryMode,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-vsx", "openvsx", mode, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

#[actix_web::test]
async fn vsix_publish_user_can_publish() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake-vsix-content".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn vsix_publish_duplicate_returns_409() {
    let app = make_local_vsx_app(RegistryMode::Local).await;

    let payload = b"PK\x03\x04fake-vsix".to_vec();
    for _ in 0..2 {
        let req = TestRequest::put()
            .uri("/proxy/local-vsx/pub.ext/0.1.0/vsix")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .insert_header(("Content-Type", "application/octet-stream"))
            .set_payload(payload.clone())
            .to_request();
        call_service(&app, req).await;
    }

    let req = TestRequest::put()
        .uri("/proxy/local-vsx/pub.ext/0.1.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(payload)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn vsix_publish_anonymous_returns_403() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn vsix_publish_proxy_mode_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::put()
        .uri("/proxy/openvsx/my-org.my-ext/1.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(b"PK\x03\x04fake".to_vec())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn vsix_download_returns_artifact_after_publish() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let vsix_bytes = b"PK\x03\x04fake-vsix-bytes".to_vec();

    let req = TestRequest::put()
        .uri("/proxy/local-vsx/my-org.my-ext/2.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(vsix_bytes.clone())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-vsx/my-org.my-ext/2.0.0/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), vsix_bytes.as_slice());
}

#[actix_web::test]
async fn vsix_download_unknown_version_returns_404() {
    let app = make_local_vsx_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-vsx/no-pub.no-ext/9.9.9/vsix")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}
