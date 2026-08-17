//! Blocking a Go module version hides it from `@v/list` and re-resolves
//! `@latest`.
//!
//! `go get module@latest` reads `@latest`; `go get module@v1.x` resolves
//! against `@v/list`. Leaving a blocked version in either makes the resolver
//! pick it, write it into `go.mod`, and only then hit the download gate — a
//! poisoned module graph rather than a policy decision.
//!
//! `@v/list` is the one listing in this codebase that is plain text rather than
//! a structured document, and `@latest` is the one that names a single version
//! and carries no list, so it is repaired by composing the two.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::call_service;
use serde_json::Value;

const MODULE: &str = "github.com/acme/widget";

async fn app() -> impl TestService {
    proxy_registry_app("local-go", "goproxy").await
}

async fn version_list<S: TestService>(app: &S, module: &str) -> String {
    get_text(app, &format!("/proxy/local-go/{module}@v/list")).await
}

async fn latest<S: TestService>(app: &S, module: &str) -> Value {
    get_json(app, &format!("/proxy/local-go/{module}@latest")).await
}

#[actix_web::test]
async fn proxy_version_list_hides_a_blocked_version() {
    let app = app().await;

    assert_eq!(
        version_list(&app, MODULE).await,
        "v1.0.0\nv1.1.0\nv2.0.0-beta.1\n"
    );

    block_version(&app, "local-go", MODULE, "v1.1.0").await;

    assert_eq!(
        version_list(&app, MODULE).await,
        "v1.0.0\nv2.0.0-beta.1\n",
        "the line is gone and the rest of the body is byte-identical"
    );
}

/// Go's `v` prefix names the same release either way, so a block recorded
/// without it must still hide the prefixed listing.
#[actix_web::test]
async fn proxy_version_list_matches_across_go_version_spellings() {
    let app = app().await;

    block_version(&app, "local-go", MODULE, "1.1.0").await;

    assert!(
        !version_list(&app, MODULE).await.contains("v1.1.0"),
        "the unprefixed block did not match the prefixed listing"
    );
}

/// `@latest` carries no list, so hiding a blocked release from it is
/// re-resolution: the newest version that survived filtering, not the newest
/// version upstream knows about.
#[actix_web::test]
async fn proxy_latest_re_resolves_off_a_blocked_version() {
    let app = app().await;

    assert_eq!(latest(&app, MODULE).await["Version"], "v1.1.0");

    block_version(&app, "local-go", MODULE, "v1.1.0").await;

    let after = latest(&app, MODULE).await;
    assert_eq!(
        after["Version"], "v1.0.0",
        "the newest allowed stable release, not the newer pre-release"
    );
    assert!(
        after.get("Time").is_none(),
        "the timestamp belonged to the release being hidden"
    );
}

#[actix_web::test]
async fn proxy_latest_naming_an_allowed_version_keeps_its_timestamp() {
    let app = app().await;

    block_version(&app, "local-go", MODULE, "v1.0.0").await;

    let after = latest(&app, MODULE).await;
    assert_eq!(after["Version"], "v1.1.0");
    assert_eq!(after["Time"], "2020-02-01T00:00:00Z");
}

/// With nothing left to name, `@latest` is a `404` — which is what the Go
/// client already handles for a module with no releases.
#[actix_web::test]
async fn proxy_latest_with_every_version_blocked_is_not_found() {
    let app = app().await;

    for v in ["v1.0.0", "v1.1.0", "v2.0.0-beta.1"] {
        block_version(&app, "local-go", MODULE, v).await;
    }

    let req = admin_get(&format!("/proxy/local-go/{MODULE}@latest"));
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn proxy_version_list_is_plain_text() {
    let app = app().await;

    assert_content_type(
        &app,
        &format!("/proxy/local-go/{MODULE}@v/list"),
        "text/plain",
    )
    .await;
}

#[actix_web::test]
async fn proxy_another_module_is_untouched() {
    let app = app().await;

    block_version(&app, "local-go", MODULE, "v1.1.0").await;

    assert!(version_list(&app, "github.com/acme/other")
        .await
        .contains("v1.1.0"));
}

/// Hiding governs resolution, not diagnosis: a `go.mod` that already pins the
/// blocked version gets the operator's `403` and reason.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-go", MODULE, "v1.1.0").await;

    let req = admin_get(&format!("/proxy/local-go/{MODULE}@v/v1.1.0.zip"));
    assert_eq!(call_service(&app, req).await.status(), 403);
}
