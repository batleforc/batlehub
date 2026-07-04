//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, init_service, read_body_json, TestRequest};
use actix_web::App;
use serde_json::Value;
use utoipa_actix_web::AppExt;

use batlehub_adapters::db::InMemoryUserBlockRepository;
use batlehub_core::ports::UserBlockRepository;
use batlehub_web::{
    AuthMiddlewareFactory, RegistryModeMap, RepoSignerMap, UserBlockMiddlewareFactory,
};

// ── /api/v1/admin/users — user block management ───────────────────────────────

/// Build a minimal app with the user block handlers, auth middleware, AND
/// `UserBlockMiddlewareFactory` so that tests can verify blocked users receive 401.
async fn make_app_with_user_block_repo(
    user_block_repo: Arc<dyn UserBlockRepository>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let EmptyAppParts {
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        cargo_indexes,
        local_svc,
    } = empty_app_parts();

    let (app, _) = App::new()
        .into_utoipa_app()
        .configure(configure_test_app(
            proxy_svc,
            admin_svc,
            token_repo,
            access_config,
            registry_map,
            ConfigureAppDefaults::default(),
        ))
        .split_for_parts();
    let app = app
        .app_data(actix_web::web::Data::new(cargo_indexes))
        .app_data(actix_web::web::Data::new(local_svc))
        .app_data(actix_web::web::Data::new(RegistryModeMap::default()))
        .app_data(actix_web::web::Data::new(RepoSignerMap::default()))
        .app_data(actix_web::web::Data::new(batlehub_web::VulnDbMap::default()))
        .app_data(actix_web::web::Data::new(Arc::clone(&user_block_repo)));

    // UserBlock added before Auth in code → UserBlock runs after Auth in the pipeline.
    init_service(
        app.wrap(UserBlockMiddlewareFactory::new(user_block_repo))
            .wrap(AuthMiddlewareFactory::new(test_auth_providers())),
    )
    .await
}

#[actix_web::test]
async fn user_blocks_list_empty_returns_200_with_empty_array() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn user_blocks_list_requires_admin() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn user_blocks_block_user_returns_204() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"reason": "spammer"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);
}

#[actix_web::test]
async fn user_blocks_block_user_requires_admin() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn user_blocks_list_shows_blocked_user() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(Arc::clone(&repo)).await;

    // Block alice
    let req = TestRequest::post()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({"reason": "test block"}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    // Verify list contains alice
    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    let list = body.as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["user_id"], "alice");
    assert_eq!(list[0]["reason"], "test block");
}

#[actix_web::test]
async fn user_blocks_unblock_removes_user() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(Arc::clone(&repo)).await;

    // Block then unblock alice
    let req = TestRequest::post()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    let req = TestRequest::delete()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 204);

    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    let body: Value = read_body_json(resp).await;
    assert_eq!(body, serde_json::json!([]));
}

#[actix_web::test]
async fn user_blocks_block_empty_user_id_returns_400() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/users/%20/block") // URL-encoded space
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn user_blocks_unblock_requires_admin() {
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    let app = make_app_with_user_block_repo(repo).await;
    let req = TestRequest::delete()
        .uri("/api/v1/admin/users/alice/block")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

#[actix_web::test]
async fn user_blocks_blocked_user_gets_401() {
    // USER_TOKEN authenticates as "user-1"; block that ID, then verify middleware rejects it.
    let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
    repo.block("user-1", "admin", Some("integration test"))
        .await
        .unwrap();
    let app = make_app_with_user_block_repo(Arc::clone(&repo)).await;

    // Blocked user is rejected before reaching any handler.
    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 401);

    // Admin (not blocked) can still access the same endpoint.
    let req = TestRequest::get()
        .uri("/api/v1/admin/users/blocked")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}
