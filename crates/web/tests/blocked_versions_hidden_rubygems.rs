//! Blocking a gem version hides it from the versions API *and* moves the gem
//! document off it.
//!
//! Two documents, read on different paths. `bundle install` resolves a
//! constraint against `/api/v1/versions/{name}.json`; `gem info` and every UI
//! read `/api/v1/gems/{name}.json`, which describes the gem at exactly one
//! version and so has to be rebuilt around a different one rather than filtered.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::Value;

async fn app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-gems", "rubygems", RegistryMode::Proxy, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

async fn get_json<S>(app: &S, uri: &str) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    read_body_json(call_service(app, req).await).await
}

fn numbers(doc: &Value) -> Vec<String> {
    doc.as_array()
        .expect("versions is an array")
        .iter()
        .map(|e| e["number"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[actix_web::test]
async fn proxy_versions_api_hides_a_blocked_version() {
    let app = app().await;
    let uri = "/proxy/local-gems/api/v1/versions/rails.json";

    assert_eq!(
        numbers(&get_json(&app, uri).await),
        ["2.0.0-beta.1", "1.1.0", "1.0.0"]
    );

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    assert_eq!(
        numbers(&get_json(&app, uri).await),
        ["2.0.0-beta.1", "1.0.0"],
        "the blocked version is gone and the newest-first order survives"
    );
}

/// The gem document *is* one version. Blocking that version has to move the
/// document to the newest one an operator does allow, or `gem info` keeps
/// naming a release the download gate will refuse.
#[actix_web::test]
async fn proxy_gem_document_moves_off_a_blocked_version() {
    let app = app().await;
    let uri = "/proxy/local-gems/api/v1/gems/rails.json";

    assert_eq!(get_json(&app, uri).await["version"], "1.1.0");

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_json(&app, uri).await;
    assert_eq!(
        after["version"], "1.0.0",
        "1.1.0 is blocked and 2.0.0-beta.1 is a pre-release, so the newest \
         allowed *stable* release wins"
    );
    assert_eq!(
        after["name"], "rails",
        "gem-level fields survive the rebuild"
    );
}

/// The checksum and download URL described the hidden release. Carried onto a
/// different version they are a hash that can never match what is downloaded.
#[actix_web::test]
async fn proxy_gem_document_drops_the_hidden_release_s_own_fields() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_json(&app, "/proxy/local-gems/api/v1/gems/rails.json").await;
    assert!(after.get("sha").is_none(), "stale checksum survived");
    assert!(
        after.get("gem_uri").is_none(),
        "stale download URL survived"
    );
}

#[actix_web::test]
async fn proxy_gem_document_naming_an_allowed_version_is_untouched() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.0.0").await;

    let after = get_json(&app, "/proxy/local-gems/api/v1/gems/rails.json").await;
    assert_eq!(after["version"], "1.1.0");
    assert_eq!(after["sha"], "bbb", "nothing to repair, nothing removed");
}

#[actix_web::test]
async fn proxy_another_gem_is_untouched() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let other = get_json(&app, "/proxy/local-gems/api/v1/versions/sinatra.json").await;
    assert!(numbers(&other).contains(&"1.1.0".to_owned()));
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let req = TestRequest::get()
        .uri("/proxy/local-gems/gems/rails-1.1.0.gem")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
