//! Integration tests for `POST /api/v1/admin/access-check`.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::test::{call_service, read_body_json, TestRequest};
use chrono::Utc;
use serde_json::{json, Value};

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;
use batlehub_core::{
    entities::PackageId,
    entities::PackageStatus,
    ports::{IpBlockStore, PackageRepository, UserBlockRepository},
};

fn access_check_body(resource_type: &str, role: &str, package_name: &str) -> Value {
    json!({
        "registry": "npm",
        "package_name": package_name,
        "version": "1.0.0",
        "resource_type": resource_type,
        "role": role,
    })
}

/// An unblock time far enough out that the store never expires it mid-test.
fn far_future() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3_600
}

async fn post_access_check<S>(app: &S, body: Value) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::post()
        .uri("/api/v1/admin/access-check")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(body)
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200);
    read_body_json(resp).await
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

// ── Block layers (RFC 0004-bis A1) ───────────────────────────────────────────
//
// The simulator called `evaluate_and_trace(&policy.rules, &ctx)` and nothing
// else. `UserBlockMiddleware` and `IpBlockMiddleware` both reject *before* any
// rule is evaluated, so an admin who blocked `alice` on `/admin/security/blocks`
// and simulated `alice` on the next tab was told **allow** — the page whose
// entire purpose is "would this identity be allowed" answering something the
// section it lives in contradicts.
//
// Every one of these has its absence assertion beside it. A new check that
// denies whenever it fires is not obviously distinguishable from a blanket
// denial until something proves the allow path still allows.

#[actix_web::test]
async fn denies_a_blocked_account_and_names_the_layer() {
    let blocks = Arc::new(InMemoryUserBlockRepository::new()) as Arc<dyn UserBlockRepository>;
    blocks
        .block("alice", "admin", Some("offboarded"))
        .await
        .unwrap();

    let app = make_app_with_blocks(blocks, Arc::new(InMemoryIpBlockStore::new())).await;
    let mut body = access_check_body("releases:read", "user", "lodash");
    body["user_id"] = json!("alice");

    let resp = post_access_check(&app, body).await;
    assert_eq!(resp["decision"], "deny");
    assert_eq!(resp["blocked_by"], "account");
    // Not a rule — nothing in the chain fired, and saying "rbac" here would send
    // the operator to the registry's policy for a decision that is not in it.
    assert!(resp["rule_matched"].is_null());
    assert!(resp["reason"].as_str().unwrap().contains("alice"));
    assert_eq!(resp["covers"]["account_blocks"], true);
}

#[actix_web::test]
async fn allows_an_unblocked_account_with_no_matching_rule() {
    let blocks = Arc::new(InMemoryUserBlockRepository::new()) as Arc<dyn UserBlockRepository>;
    blocks.block("mallory", "admin", None).await.unwrap();

    let app = make_app_with_blocks(blocks, Arc::new(InMemoryIpBlockStore::new())).await;
    let mut body = access_check_body("releases:read", "user", "lodash");
    body["user_id"] = json!("alice"); // a *different* account is blocked

    let resp = post_access_check(&app, body).await;
    assert_eq!(resp["decision"], "allow");
    assert!(resp["blocked_by"].is_null());
}

#[actix_web::test]
async fn denies_a_blocked_ip_and_names_the_layer() {
    let ips = Arc::new(InMemoryIpBlockStore::new()) as Arc<dyn IpBlockStore>;
    ips.block_ip("10.0.0.7", far_future(), "brute force")
        .await
        .unwrap();

    let app = make_app_with_blocks(Arc::new(InMemoryUserBlockRepository::new()), ips).await;
    let mut body = access_check_body("releases:read", "user", "lodash");
    body["client_ip"] = json!("10.0.0.7");

    let resp = post_access_check(&app, body).await;
    assert_eq!(resp["decision"], "deny");
    assert_eq!(resp["blocked_by"], "ip");
    assert_eq!(resp["covers"]["ip_blocks"], true);
}

#[actix_web::test]
async fn allows_an_unblocked_ip() {
    let ips = Arc::new(InMemoryIpBlockStore::new()) as Arc<dyn IpBlockStore>;
    ips.block_ip("10.0.0.7", far_future(), "brute force")
        .await
        .unwrap();

    let app = make_app_with_blocks(Arc::new(InMemoryUserBlockRepository::new()), ips).await;
    let mut body = access_check_body("releases:read", "user", "lodash");
    body["client_ip"] = json!("10.0.0.8");

    let resp = post_access_check(&app, body).await;
    assert_eq!(resp["decision"], "allow");
}

/// B4: a simulation with no address must not answer as if it had one.
///
/// The endpoint cannot check the IP layer without an IP, and returning `allow`
/// because none was supplied reproduces the original defect one level down. So
/// it says which layers the answer accounts for, and `ip_blocks` is false here.
#[actix_web::test]
async fn states_that_it_did_not_check_the_layers_it_had_no_input_for() {
    let app = make_app(InMemoryRepo::new()).await;
    let resp = post_access_check(&app, access_check_body("releases:read", "user", "lodash")).await;

    assert_eq!(resp["decision"], "allow");
    assert_eq!(resp["covers"]["rules"], true);
    assert_eq!(resp["covers"]["account_blocks"], false);
    assert_eq!(resp["covers"]["ip_blocks"], false);
}

/// The IP layer rejects first in production, so it must here — otherwise
/// `blocked_by` tells an operator to go and unblock the account when the
/// address is what a real request would have hit.
#[actix_web::test]
async fn reports_the_ip_layer_when_both_would_block() {
    let blocks = Arc::new(InMemoryUserBlockRepository::new()) as Arc<dyn UserBlockRepository>;
    blocks.block("alice", "admin", None).await.unwrap();
    let ips = Arc::new(InMemoryIpBlockStore::new()) as Arc<dyn IpBlockStore>;
    ips.block_ip("10.0.0.7", far_future(), "brute force")
        .await
        .unwrap();

    let app = make_app_with_blocks(blocks, ips).await;
    let mut body = access_check_body("releases:read", "user", "lodash");
    body["user_id"] = json!("alice");
    body["client_ip"] = json!("10.0.0.7");

    let resp = post_access_check(&app, body).await;
    assert_eq!(resp["blocked_by"], "ip");
}

/// A rule deny still reads as a rule deny, and still names the rule.
#[actix_web::test]
async fn a_rule_deny_is_attributed_to_the_rule_layer() {
    let app = make_app(InMemoryRepo::new()).await;
    let resp = post_access_check(
        &app,
        access_check_body("source:read", "anonymous", "lodash"),
    )
    .await;

    assert_eq!(resp["decision"], "deny");
    assert_eq!(resp["blocked_by"], "rule");
    assert_eq!(resp["rule_matched"], "rbac");
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
