//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_config::schema::RegistryMode;

// ── packages.json ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_packages_json_proxy_mode_returns_metadata_url() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/packages.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let metadata_url = body["metadata-url"].as_str().unwrap();
    assert!(
        metadata_url.contains("/proxy/local-composer/p2/%package%.json"),
        "metadata-url must point to our p2 endpoint"
    );
    assert_eq!(body["available-packages"], serde_json::json!([]));
}

#[actix_web::test]
async fn composer_packages_json_local_mode_lists_published_packages() {
    let app = make_local_composer_app(RegistryMode::Local).await;

    // Publish a package first so it appears in the listing.
    let zip = make_composer_zip("acme/my-pkg", "1.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);

    let req = TestRequest::get()
        .uri("/proxy/local-composer/packages.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let available = body["available-packages"].as_array().unwrap();
    assert!(
        available.iter().any(|v| v.as_str() == Some("acme/my-pkg")),
        "available-packages must list published package name"
    );
}

#[actix_web::test]
async fn composer_packages_json_unknown_registry_returns_404() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/no-such-registry/packages.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── p2 metadata ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_p2_proxy_mode_returns_artifact_body() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/vendor/pkg.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    // FixedRegistry returns "artifact:composer:…" — assert content originates from the registry call
    let body_str = std::str::from_utf8(&body).expect("body is valid UTF-8");
    assert!(
        body_str.contains("vendor/pkg"),
        "response body must reference the requested package name; got: {body_str:?}"
    );
}

#[actix_web::test]
async fn composer_p2_dev_variant_returns_200_and_body() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/vendor/pkg~dev.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    // ~dev.json is a valid variant — the parse helper strips the suffix.
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    let body_str = std::str::from_utf8(&body).expect("body is valid UTF-8");
    assert!(
        body_str.contains("vendor/pkg"),
        "response body must reference the requested package name; got: {body_str:?}"
    );
}

#[actix_web::test]
async fn composer_p2_local_mode_published_package_found() {
    let app = make_local_composer_app(RegistryMode::Local).await;

    let zip = make_composer_zip("acme/my-lib", "2.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/acme/my-lib.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(body["packages"]["acme/my-lib"].is_array());
}

#[actix_web::test]
async fn composer_p2_local_mode_unknown_package_returns_404() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/ghost/pkg.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn composer_p2_hybrid_mode_falls_back_to_proxy() {
    // In hybrid mode with no local packages the request falls back to FixedRegistry.
    let app = make_local_composer_app(RegistryMode::Hybrid).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/vendor/remote-pkg.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── dist artifact ─────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_dist_proxy_mode_streams_artifact() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/dist/vendor/pkg/1.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn composer_dist_local_mode_serves_stored_artifact() {
    let app = make_local_composer_app(RegistryMode::Local).await;

    let zip = make_composer_zip("acme/zippkg", "3.1.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip.clone())
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let req = TestRequest::get()
        .uri("/proxy/local-composer/dist/acme/zippkg/3.1.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = read_body(resp).await;
    assert_eq!(body.as_ref(), zip.as_slice());
}

#[actix_web::test]
async fn composer_dist_local_mode_unknown_version_returns_404() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/dist/ghost/pkg/9.9.9")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn composer_dist_hybrid_falls_back_to_proxy() {
    let app = make_local_composer_app(RegistryMode::Hybrid).await;
    let req = TestRequest::get()
        .uri("/proxy/local-composer/dist/vendor/remote/1.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── upload ────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_upload_user_can_publish() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let zip = make_composer_zip("myvendor/mypkg", "1.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["status"], "success");
    assert_eq!(body["name"], "myvendor/mypkg");
    assert_eq!(body["version"], "1.0.0");
}

#[actix_web::test]
async fn composer_upload_version_override_via_query_param() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    // ZIP has version "1.0.0" in composer.json but we override to "2.5.0".
    let zip = make_composer_zip("myvendor/override-pkg", "1.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload?version=2.5.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["version"], "2.5.0");
}

#[actix_web::test]
async fn composer_upload_anonymous_returns_403() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let zip = make_composer_zip("myvendor/anon-pkg", "1.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        // No Authorization header — anonymous identity.
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn composer_upload_proxy_mode_returns_404() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let zip = make_composer_zip("myvendor/proxy-pkg", "1.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn composer_upload_duplicate_version_returns_409() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let zip = make_composer_zip("myvendor/dup-pkg", "1.0.0");

    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip.clone())
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn composer_upload_invalid_zip_returns_422() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"this is not a zip file".as_slice())
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 422);
}

#[actix_web::test]
async fn composer_upload_then_p2_shows_package() {
    let app = make_local_composer_app(RegistryMode::Local).await;

    let zip = make_composer_zip("acme/seq-pkg", "1.2.3");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/acme/seq-pkg.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let versions = body["packages"]["acme/seq-pkg"].as_array().unwrap();
    assert!(!versions.is_empty());
    assert_eq!(versions[0]["version"], "1.2.3");
    assert!(versions[0]["dist"]["url"]
        .as_str()
        .unwrap()
        .contains("/proxy/local-composer/dist/acme/seq-pkg/1.2.3"));
}

// ── yank ──────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_yank_excludes_version_from_p2() {
    // Yanked versions are removed from the Packagist v2 response because Composer
    // clients have no standard `yanked` field — they would otherwise install yanked releases.
    let app = make_local_composer_app(RegistryMode::Local).await;

    let zip = make_composer_zip("acme/yankable", "4.0.0");
    let req = TestRequest::post()
        .uri("/proxy/local-composer/api/upload")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(zip)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Verify the version appears before yanking.
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/acme/yankable.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(!body["packages"]["acme/yankable"]
        .as_array()
        .unwrap()
        .is_empty());

    let req = TestRequest::delete()
        .uri("/proxy/local-composer/api/packages/acme/yankable/versions/4.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // After yanking the only version, the p2 endpoint should return 404.
    let req = TestRequest::get()
        .uri("/proxy/local-composer/p2/acme/yankable.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn composer_yank_anonymous_returns_403() {
    let app = make_local_composer_app(RegistryMode::Local).await;
    let req = TestRequest::delete()
        .uri("/proxy/local-composer/api/packages/acme/anon-pkg/versions/1.0.0")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn composer_yank_proxy_mode_returns_404() {
    let app = make_local_composer_app(RegistryMode::Proxy).await;
    let req = TestRequest::delete()
        .uri("/proxy/local-composer/api/packages/acme/proxy-pkg/versions/1.0.0")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── misc ──────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn composer_wrong_registry_type_returns_404() {
    // "npm" registry exists but is type "npm", not "composer".
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/proxy/npm/packages.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}
