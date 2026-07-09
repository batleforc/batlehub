//! Integration tests for `POST /api/v1/admin/access-check`.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use chrono::Utc;
use serde_json::{json, Value};

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_core::{entities::PackageId, entities::PackageStatus, ports::PackageRepository};

fn access_check_body(resource_type: &str, role: &str, package_name: &str) -> Value {
    json!({
        "registry": "npm",
        "package_name": package_name,
        "version": "1.0.0",
        "resource_type": resource_type,
        "role": role,
    })
}

#[actix_web::test]
async fn non_admin_identity_is_forbidden() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(access_check_body("releases:read", "anonymous", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn anonymous_identity_is_forbidden() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .set_json(access_check_body("releases:read", "anonymous", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn unconfigured_registry_returns_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let mut body = access_check_body("releases:read", "anonymous", "lodash");
    body["registry"] = json!("pypi");
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(body)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

#[actix_web::test]
async fn invalid_role_returns_400() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(access_check_body("releases:read", "superuser", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn allows_anonymous_read_of_permitted_resource() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(access_check_body("releases:read", "anonymous", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["decision"], "allow");
    assert!(body["reason"].is_null());
    assert!(body["rule_matched"].is_null());
}

#[actix_web::test]
async fn denies_anonymous_read_of_user_only_resource() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(access_check_body("source:read", "anonymous", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["decision"], "deny");
    assert!(body["reason"].as_str().unwrap().contains("not permitted"));
    assert_eq!(body["rule_matched"], "rbac");
}

#[actix_web::test]
async fn allows_user_read_of_user_only_resource() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(access_check_body("source:read", "user", "lodash"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["decision"], "allow");
}

#[actix_web::test]
async fn denies_blocked_package_via_block_list_rule() {
    let repo = InMemoryRepo::new();
    let pkg = PackageId::new("npm", "evil-pkg", "1.0.0");
    repo.set_status(
        &pkg,
        PackageStatus::Blocked {
            reason: "security vulnerability".to_owned(),
            blocked_by: "admin".to_owned(),
            blocked_at: Utc::now(),
        },
    )
    .await
    .unwrap();

    let app = make_app(repo).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(access_check_body("releases:read", "admin", "evil-pkg"))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["decision"], "deny");
    assert_eq!(body["reason"], "security vulnerability");
    assert_eq!(body["rule_matched"], "block_list");
}

#[actix_web::test]
async fn defaults_to_anonymous_role_when_omitted() {
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(json!({
            "registry": "npm",
            "package_name": "lodash",
            "version": "1.0.0",
            "resource_type": "releases:read",
        }))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["decision"], "allow");
}
