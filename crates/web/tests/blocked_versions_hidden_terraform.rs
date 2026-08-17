//! Blocking a Terraform version hides it from the module and provider version
//! listings.
//!
//! `terraform init` picks a version by reading `/v1/{namespace}/versions` and
//! then downloads it. Leaving a blocked version in that listing makes the init
//! select it and fail at download, mid-plan — which reads as a broken registry
//! rather than as policy.
//!
//! One route serves two document shapes, told apart by the `modules/` or
//! `providers/` prefix, so both are covered here.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::call_service;
use batlehub_config::schema::RegistryMode;
use serde_json::Value;

const MODULE: &str = "terraform-aws-modules/vpc/aws";
const PROVIDER: &str = "hashicorp/aws";

async fn app() -> impl TestService {
    proxy_registry_app("local-tf", "terraform").await
}

fn listed(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .expect("a versions array")
        .iter()
        .map(|e| e["version"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[actix_web::test]
async fn proxy_module_versions_hide_a_blocked_version() {
    let app = app().await;
    let uri = format!("/proxy/local-tf/v1/modules/{MODULE}/versions");

    let before = get_json(&app, &uri).await;
    assert_eq!(
        listed(&before["modules"][0]["versions"]),
        ["1.0.0", "1.1.0", "2.0.0-beta.1"]
    );

    // The block is recorded against the coordinate the proxy uses internally,
    // which carries the `modules/` facet prefix.
    block_version(&app, "local-tf", &format!("modules/{MODULE}"), "1.1.0").await;

    let after = get_json(&app, &uri).await;
    assert_eq!(
        listed(&after["modules"][0]["versions"]),
        ["1.0.0", "2.0.0-beta.1"]
    );
    assert!(
        after["modules"][0]["source"].is_string(),
        "the envelope has to survive, or terraform cannot read the response"
    );
}

#[actix_web::test]
async fn proxy_provider_versions_hide_a_blocked_version() {
    let app = app().await;
    let uri = format!("/proxy/local-tf/v1/providers/{PROVIDER}/versions");

    block_version(&app, "local-tf", &format!("providers/{PROVIDER}"), "1.1.0").await;

    let after = get_json(&app, &uri).await;
    assert_eq!(listed(&after["versions"]), ["1.0.0", "2.0.0-beta.1"]);
    assert!(after["id"].is_string(), "the envelope survives");
}

#[actix_web::test]
async fn proxy_versions_response_is_json() {
    let app = app().await;

    assert_content_type(
        &app,
        &format!("/proxy/local-tf/v1/modules/{MODULE}/versions"),
        "application/json",
    )
    .await;
}

/// Blocking a module version must not touch the provider of the same name, and
/// the two facets are separate coordinates precisely so it cannot.
#[actix_web::test]
async fn blocking_a_module_version_leaves_the_provider_listing_alone() {
    let app = app().await;

    block_version(&app, "local-tf", &format!("modules/{MODULE}"), "1.1.0").await;

    let providers = get_json(
        &app,
        &format!("/proxy/local-tf/v1/providers/{PROVIDER}/versions"),
    )
    .await;
    assert!(listed(&providers["versions"]).contains(&"1.1.0".to_owned()));
}

/// Hiding governs resolution, not diagnosis: the artifact route still refuses
/// the blocked version with the operator's reason, rather than 404ing as though
/// the version had never existed.
///
/// Asserted in **local** mode because that is where Terraform transfers bytes
/// through this proxy at all: `…/{version}/download` in proxy mode resolves the
/// upstream's `X-Terraform-Get` and hands the client a URL to fetch directly,
/// which predates this work and is unchanged by it.
#[actix_web::test]
async fn direct_request_for_a_blocked_version_is_still_denied() {
    let app = registry_app("local-tf", "terraform", RegistryMode::Local).await;

    block_version(&app, "local-tf", &format!("modules/{MODULE}"), "1.1.0").await;

    let req = admin_get(&format!(
        "/proxy/local-tf/v1/modules/{MODULE}/1.1.0/artifact"
    ));
    assert_eq!(call_service(&app, req).await.status(), 403);
}
