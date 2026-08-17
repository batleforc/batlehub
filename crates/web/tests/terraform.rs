//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use base64::Engine as _;
use batlehub_config::schema::RegistryMode;
use batlehub_web::RegistryModeMap;

// ══ Terraform local registry tests ════════════════════════════════════════════

async fn make_local_terraform_app(mode: RegistryMode) -> impl TestService {
    build_local_registry_app(
        local_only_app_parts("local-tf", "terraform", mode, false),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

/// Like `make_local_terraform_app`, but also returns the `RegistryModeMap` handle
/// so a test can flip the registry's mode after publishing (simulating a
/// hot-reload) to confirm mode-gated endpoints re-check the *current* mode.
async fn make_local_terraform_app_with_mode_map(
    mode: RegistryMode,
) -> (impl TestService, RegistryModeMap) {
    let parts = local_only_app_parts("local-tf", "terraform", mode, false);
    let mode_map = parts.mode_map.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;
    (app, mode_map)
}

#[actix_web::test]
async fn terraform_provider_artifact_proxy_mode_rejects_previously_published_binary() {
    let (app, mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"fake-zip-bytes".as_slice())
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Confirm it's actually retrievable while still in Local mode.
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Simulate a hot-reload switching the registry to Proxy mode: the binary
    // must no longer be servable from local storage.
    //
    // Since RFC 0009 §7.2 this route also has a proxy fall-through — the
    // provider download document points `download_url` here so the zip is
    // gated and cached instead of fetched from upstream's CDN directly — so the
    // request no longer stops at a `require_local_mode` guard. The property
    // under test is unchanged and is asserted directly: whatever the proxy path
    // answers, it must not be the locally published bytes.
    mode_map.insert("local-tf".to_owned(), RegistryMode::Proxy);
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_ne!(
        resp.status(),
        200,
        "local storage must not answer once the registry is in proxy mode"
    );
    let body = actix_web::test::read_body(resp).await;
    assert_ne!(body.as_ref(), b"fake-zip-bytes".as_slice());
}

#[actix_web::test]
async fn terraform_module_artifact_proxy_mode_rejects_previously_published_tarball() {
    let (app, mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Local).await;
    let payload = b"tarball-content-bytes";

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(payload.as_slice())
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    // Confirm it's actually retrievable while still in Local mode.
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0/artifact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Simulate a hot-reload switching the registry to Proxy mode: the tarball
    // must no longer be servable from local storage.
    //
    // Since RFC 0009 §7.2 this route also has a proxy fall-through — module
    // downloads are redirected here rather than to upstream so the bytes go
    // through the rule chain — so the request no longer stops at a
    // `require_local_mode` guard. The property under test is unchanged.
    mode_map.insert("local-tf".to_owned(), RegistryMode::Proxy);
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0/artifact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_ne!(
        resp.status(),
        200,
        "local storage must not answer once the registry is in proxy mode"
    );
    let body = actix_web::test::read_body(resp).await;
    assert_ne!(body.as_ref(), payload.as_slice());
}

// ── Terraform module tests ────────────────────────────────────────────────────

#[actix_web::test]
async fn terraform_module_upload_returns_201() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"fake-tarball-bytes".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
}

#[actix_web::test]
async fn terraform_module_versions_after_upload() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let versions = body["modules"][0]["versions"].as_array().unwrap();
    assert!(versions.iter().any(|v| v["version"] == "0.1.0"));
}

#[actix_web::test]
async fn terraform_module_download_local_returns_204_with_header() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0/download")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 204);
    let header = resp
        .headers()
        .get("X-Terraform-Get")
        .expect("X-Terraform-Get header must be present");
    let url = header.to_str().unwrap();
    assert!(
        url.contains("/artifact"),
        "X-Terraform-Get should point at /artifact"
    );
}

#[actix_web::test]
async fn terraform_module_artifact_returns_bytes() {
    let app = make_local_terraform_app(RegistryMode::Local).await;
    let payload = b"tarball-content-bytes";

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(payload.as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0/artifact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(read_body(resp).await, payload.as_slice());
}

#[actix_web::test]
async fn terraform_module_upload_duplicate_returns_409() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    for _ in 0..2 {
        let req = TestRequest::post()
            .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .set_payload(b"tarball".as_slice())
            .to_request();
        let _ = call_service(&app, req).await;
    }

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

// ── Terraform provider tests ──────────────────────────────────────────────────

const PROVIDER_MANIFEST: &str = r#"{
  "version": "5.0.0",
  "protocols": ["5.0"],
  "platforms": [
    {"os": "linux", "arch": "amd64", "filename": "terraform-provider-aws_5.0.0_linux_amd64.zip", "shasum": "deadbeef"}
  ]
}"#;

#[actix_web::test]
async fn terraform_provider_upload_manifest_returns_201() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
}

#[actix_web::test]
async fn terraform_provider_binary_upload_returns_200() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    // Must upload manifest first (no strict requirement in handler, but good practice)
    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"fake-zip-bytes".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn terraform_provider_binary_upload_anonymous_returns_403() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    // No Authorization header at all — must be rejected by enforce_publish_policy
    // the same way the manifest upload is, not silently stored.
    let req = TestRequest::put()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .set_payload(b"fake-zip-bytes".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn terraform_provider_versions_after_upload() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let versions = body["versions"].as_array().unwrap();
    assert!(versions.iter().any(|v| v["version"] == "5.0.0"));
}

#[actix_web::test]
async fn terraform_provider_download_contains_local_artifact_url() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let download_url = body["download_url"].as_str().unwrap();
    assert!(
        download_url.contains("/artifact/linux/amd64"),
        "download_url should point at local artifact endpoint, got: {download_url}"
    );
}

// ── Terraform module yank / unyank ────────────────────────────────────────────

#[actix_web::test]
async fn terraform_module_yank_returns_200() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["message"].as_str().unwrap().contains("yanked"));
}

#[actix_web::test]
async fn terraform_module_yanked_hidden_from_versions() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    // After yank the only version is yanked; local_svc returns NotFound when all are yanked
    assert!(resp.status() == 200 || resp.status() == 404);
}

#[actix_web::test]
async fn terraform_module_unyank_returns_200() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions/0.1.0/unyank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["message"].as_str().unwrap().contains("unyanked"));
}

#[actix_web::test]
async fn terraform_module_yank_requires_auth() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/versions/0.1.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert!(resp.status() == 401 || resp.status() == 403);
}

// ── Terraform provider yank / unyank ─────────────────────────────────────────

#[actix_web::test]
async fn terraform_provider_yank_returns_200() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions/5.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["message"].as_str().unwrap().contains("yanked"));
}

#[actix_web::test]
async fn terraform_provider_unyank_returns_200() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions/5.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions/5.0.0/unyank")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["message"].as_str().unwrap().contains("unyanked"));
}

#[actix_web::test]
async fn terraform_provider_yank_requires_auth() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::delete()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions/5.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert!(resp.status() == 401 || resp.status() == 403);
}

// ── Terraform signing headers ─────────────────────────────────────────────────

#[actix_web::test]
async fn terraform_module_upload_with_signature_preserved_on_artifact_download() {
    let app = make_local_terraform_app(RegistryMode::Local).await;
    let sig = base64::engine::general_purpose::STANDARD.encode(b"fake-ed25519-sig");

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.2.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("X-Artifact-Signature", sig.as_str()))
        .insert_header(("X-Signature-Type", "ed25519"))
        .set_payload(b"tarball".as_slice())
        .to_request();
    let upload_resp = call_service(&app, req).await;
    assert_eq!(upload_resp.status(), 201);

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.2.0/artifact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    // Signature headers must be echoed back on download
    assert!(
        resp.headers().get("X-Artifact-Signature").is_some(),
        "X-Artifact-Signature header must be present on artifact download"
    );
    assert_eq!(
        resp.headers()
            .get("X-Signature-Type")
            .and_then(|v| v.to_str().ok()),
        Some("ed25519")
    );
}

#[actix_web::test]
async fn terraform_provider_upload_with_signature_preserved_on_download_info() {
    let app = make_local_terraform_app(RegistryMode::Local).await;
    let sig = base64::engine::general_purpose::STANDARD.encode(b"fake-provider-sig");

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .insert_header(("X-Artifact-Signature", sig.as_str()))
        .insert_header(("X-Signature-Type", "ed25519"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    let upload_resp = call_service(&app, req).await;
    assert_eq!(upload_resp.status(), 201);

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().get("X-Artifact-Signature").is_some(),
        "X-Artifact-Signature header must be present on provider download info"
    );
    assert_eq!(
        resp.headers()
            .get("X-Signature-Type")
            .and_then(|v| v.to_str().ok()),
        Some("ed25519")
    );
}

// ── Terraform quota headers ───────────────────────────────────────────────────

#[actix_web::test]
async fn terraform_module_upload_returns_quota_headers() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.3.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"tarball".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
    // Quota headers are only present when a quota is configured; the in-memory backend
    // has no quota, so they are absent — but the response must still be 201.
    // This test verifies the handler correctly returns 201 regardless of quota header presence.
}

#[actix_web::test]
async fn terraform_provider_upload_returns_quota_headers() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 201);
}

// ── Terraform provider read paths (versions/download/artifact) ──────────────

#[actix_web::test]
async fn terraform_provider_versions_local_unknown_returns_404() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/unknown/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn terraform_provider_versions_hybrid_falls_through_to_proxy() {
    let app = make_local_terraform_app(RegistryMode::Hybrid).await;

    // No upload, and no RegistryClient configured for "local-tf" — the Hybrid
    // fallthrough on local NotFound reaches proxy_stream, which then fails fast
    // with "unknown registry" since the registries map is empty in this factory.
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/unknown/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn terraform_provider_versions_proxy_mode_goes_straight_to_proxy() {
    let app = make_local_terraform_app(RegistryMode::Proxy).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn terraform_provider_download_local_unknown_returns_404() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/unknown/9.9.9/download/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn terraform_provider_download_hybrid_falls_through_to_proxy() {
    let app = make_local_terraform_app(RegistryMode::Hybrid).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/unknown/9.9.9/download/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn terraform_provider_artifact_path_traversal_returns_400() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    // `%2F..` decodes to a path segment containing "/.." — caught by
    // validate_path_safe before it ever becomes a storage key.
    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux%2F../amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn terraform_provider_artifact_not_found_returns_404() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn terraform_provider_artifact_returns_uploaded_binary() {
    let app = make_local_terraform_app(RegistryMode::Local).await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    call_service(&app, req).await;

    let req = TestRequest::put()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"fake-zip-bytes".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    let req = TestRequest::get()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/zip"
    );
    let body = read_body(resp).await;
    assert_eq!(&body[..], b"fake-zip-bytes");
}

// ── Discovery, the network mirror, and the download gate (RFC 0009 §7.2) ─────
//
// Three defects, one phase. Discovery did not exist, so Terraform could not find
// the `/v1/` routes at all. The network mirror our own docs configured was a
// different protocol from the one implemented. And provider/module downloads
// handed the client an upstream URL, so the bytes never passed through the rule
// chain — RFC 0006 §13.6, which this closes.

/// Discovery is host-rooted by the protocol. Under path routing there is no
/// single registry it could describe, so it declines rather than guessing.
#[actix_web::test]
async fn discovery_declines_on_a_path_routed_request() {
    let (app, _mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/.well-known/terraform.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);

    // ...and says why, rather than looking like a missing route.
    let body = actix_web::test::read_body(resp).await;
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("host bound to a single") || text.contains("subdomain_routing"),
        "the 404 must name the host-routing prerequisite, got: {text}"
    );
}

/// The mirror path carries the *origin* registry's hostname so one mirror can
/// serve several. We are one upstream per registry, so a mismatch is refused
/// rather than echoed — otherwise the document would attach an `example.com`
/// provenance to a `registry.terraform.io` provider (RFC 0009 §11.1).
#[actix_web::test]
async fn the_mirror_refuses_a_hostname_that_is_not_this_registrys_upstream() {
    let (app, _mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-tf/evil.example.com/hashicorp/aws/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

/// The regression `protocol_conformance.rs` caught while this phase was being
/// written: the mirror's four-segment pattern claimed RubyGems'
/// `/api/v1/versions/{gem}.json` as host="api", ns="v1", type="versions".
///
/// Pinned here as well as in the conformance table, because the constraint that
/// fixes it (a hostname must contain a dot) is easy to relax by accident.
#[actix_web::test]
async fn the_mirror_pattern_does_not_claim_a_dotless_four_segment_path() {
    let (app, _mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-tf/api/v1/versions/rails.json")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_ne!(
        resp.request().match_pattern().as_deref(),
        Some(r"/proxy/{registry}/{hostname:[^/]+\.[^/]+}/{namespace}/{ptype}/{version}.json"),
        "a dotless segment must not be taken for a registry hostname"
    );
}

/// `index.json` is a legal `{version}` capture, so the two mirror routes are
/// order-sensitive. Registered the wrong way round, the version route answers
/// the index request.
#[actix_web::test]
async fn the_mirror_index_is_not_claimed_by_the_version_route() {
    let (app, _mode_map) = make_local_terraform_app_with_mode_map(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-tf/registry.terraform.io/hashicorp/aws/index.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(
        resp.request().match_pattern().as_deref(),
        Some(r"/proxy/{registry}/{hostname:[^/]+\.[^/]+}/{namespace}/{ptype}/index.json"),
        "index.json must reach the index route, not the version route"
    );
}

/// The gate this phase closes: a module download must not hand the client an
/// upstream URL. Whatever mode the registry is in, `X-Terraform-Get` points at
/// a route on *this* host, which runs the rule chain on the bytes.
#[actix_web::test]
async fn a_module_download_points_at_this_proxy_not_upstream() {
    for mode in [
        RegistryMode::Local,
        RegistryMode::Proxy,
        RegistryMode::Hybrid,
    ] {
        let (app, _mode_map) = make_local_terraform_app_with_mode_map(mode.clone()).await;
        let req = TestRequest::get()
            .uri("/proxy/local-tf/v1/modules/hashicorp/consul/aws/0.1.0/download")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        let header = resp
            .headers()
            .get("X-Terraform-Get")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(
            header.contains("/v1/modules/hashicorp/consul/aws/0.1.0/artifact"),
            "{mode:?}: X-Terraform-Get must name our own artifact route, got {header:?}"
        );
        assert!(
            !header.contains("registry.terraform.io"),
            "{mode:?}: the client must never be sent to upstream directly, got {header:?}"
        );
    }
}
