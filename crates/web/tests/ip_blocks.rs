//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::rate_limit::InMemoryIpBlockStore;
use batlehub_core::ports::IpBlockStore;

// ── /api/v1/admin/ip-blocks ───────────────────────────────────────────────────

#[actix_web::test]
async fn ip_blocks_list_empty_returns_200_with_empty_array() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn ip_blocks_block_ip_returns_204() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"ip": "1.2.3.4"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 204);
}

#[actix_web::test]
async fn ip_blocks_list_shows_blocked_ip() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(Arc::clone(&store)).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"ip": "10.0.0.1", "reason": "spam", "duration_secs": 3600}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    let req = TestRequest::get()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["ip"], "10.0.0.1");
    assert_eq!(list[0]["reason"], "spam");
}

#[actix_web::test]
async fn ip_blocks_unblock_ip_returns_204_and_removes_from_list() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(Arc::clone(&store)).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"ip": "5.6.7.8"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    let req = TestRequest::delete()
        .uri("/api/v1/admin/ip-blocks/5.6.7.8")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    let req = TestRequest::get()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn ip_blocks_block_invalid_ip_returns_400() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"ip": "not-an-ip"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn ip_blocks_block_zero_duration_returns_400() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"ip": "1.2.3.4", "duration_secs": 0}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

#[actix_web::test]
async fn ip_blocks_requires_admin() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({"ip": "1.2.3.4"}))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

// ── IP blocks — list ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn ip_blocks_list_requires_admin() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn ip_blocks_list_returns_empty_initially() {
    let store: Arc<dyn IpBlockStore> = Arc::new(InMemoryIpBlockStore::new());
    let app = make_app_with_ip_store(store).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/ip-blocks")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}
