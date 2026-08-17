//! Blocking a Maven version hides it from `maven-metadata.xml` and repairs the
//! two pointers the document carries.
//!
//! Maven resolves a range, `LATEST` and `RELEASE` against this one document, so
//! a blocked version left in `<versions>` gets selected and then refused —
//! mid-build, after the reactor has already committed to it.
//!
//! It is also the only listing here in XML and the only one that names *two*
//! preferred versions: `<latest>` may be a snapshot, `<release>` may not.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::call_service;

const COORD: &str = "com/example/mylib";
const NAME: &str = "com.example:mylib";

async fn app() -> impl TestService {
    proxy_registry_app("local-mvn", "maven").await
}

async fn metadata<S: TestService>(app: &S) -> String {
    get_text(
        app,
        &format!("/proxy/local-mvn/maven2/{COORD}/maven-metadata.xml"),
    )
    .await
}

#[actix_web::test]
async fn proxy_metadata_hides_a_blocked_version() {
    let app = app().await;

    assert!(metadata(&app).await.contains("<version>1.1.0</version>"));

    block_version(&app, "local-mvn", NAME, "1.1.0").await;

    let after = metadata(&app).await;
    assert!(
        !after.contains("<version>1.1.0</version>"),
        "blocked version still listed: {after}"
    );
    assert!(after.contains("<version>1.0.0</version>"));
}

/// The distinction the two elements exist to draw: `<latest>` may name a
/// qualified version, `<release>` may not. Repairing them the same way would
/// hand `RELEASE` a beta.
#[actix_web::test]
async fn proxy_metadata_repairs_latest_and_release_differently() {
    let app = app().await;

    block_version(&app, "local-mvn", NAME, "1.1.0").await;

    let after = metadata(&app).await;
    assert!(
        after.contains("<release>1.0.0</release>"),
        "release must skip the beta: {after}"
    );
    assert!(
        after.contains("<latest>2.0.0-beta.1</latest>"),
        "latest may name the beta: {after}"
    );
}

#[actix_web::test]
async fn proxy_metadata_is_xml() {
    let app = app().await;

    // `contains` rather than `starts_with`: Maven is served as
    // `application/xml` or `text/xml` depending on what upstream declared.
    let resp = call_service(
        &app,
        admin_get(&format!(
            "/proxy/local-mvn/maven2/{COORD}/maven-metadata.xml"
        )),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .expect("content-type set")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(ct.contains("xml"), "content-type was {ct}");
}

#[actix_web::test]
async fn proxy_metadata_with_nothing_blocked_is_byte_identical_to_upstream() {
    let app = app().await;

    let before = metadata(&app).await;
    block_version(&app, "local-mvn", "com.example:other", "1.1.0").await;

    assert_eq!(
        metadata(&app).await,
        before,
        "another artifact's block must not reformat this document"
    );
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-mvn", NAME, "1.1.0").await;

    let req = admin_get(&format!(
        "/proxy/local-mvn/maven2/{COORD}/1.1.0/mylib-1.1.0.jar"
    ));
    assert_eq!(call_service(&app, req).await.status(), 403);
}
