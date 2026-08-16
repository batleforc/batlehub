//! Blocking a PyPI version hides every file of that version from the simple
//! index, in both of its representations.
//!
//! pip resolves a requirement by reading `/simple/{package}/` and picking a
//! file. A blocked version left there gets chosen, written into a lockfile, and
//! refused at download.
//!
//! Two things make PyPI the awkward one. The index lists **files**, not
//! versions, so a version is recovered from each distribution filename — and a
//! filename this proxy cannot parse is kept, because over-listing one file
//! beats hiding a package's whole file set. And one URL serves two documents:
//! PEP 503 HTML and PEP 691 JSON, chosen by `Accept`.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::Value;

const JSON_ACCEPT: &str = "application/vnd.pypi.simple.v1+json";

async fn app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-pypi", "pypi", RegistryMode::Proxy, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

async fn simple_html<S>(app: &S, package: &str) -> String
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!("/proxy/local-pypi/simple/{package}/"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body = read_body(call_service(app, req).await).await;
    String::from_utf8(body.to_vec()).expect("a simple page is UTF-8")
}

async fn simple_json<S>(app: &S, package: &str) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!("/proxy/local-pypi/simple/{package}/"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Accept", JSON_ACCEPT))
        .to_request();
    read_body_json(call_service(app, req).await).await
}

#[actix_web::test]
async fn proxy_simple_html_hides_a_blocked_version() {
    let app = app().await;

    assert!(simple_html(&app, "requests")
        .await
        .contains("requests-1.1.0"));

    block_version(&app, "local-pypi", "requests", "1.1.0").await;

    let after = simple_html(&app, "requests").await;
    assert!(
        !after.contains("requests-1.1.0"),
        "blocked version still linked: {after}"
    );
    assert!(after.contains("requests-1.0.0"));
    assert!(after.contains("</body></html>"), "the envelope survives");
}

#[actix_web::test]
async fn proxy_simple_json_hides_a_blocked_version() {
    let app = app().await;

    block_version(&app, "local-pypi", "requests", "1.1.0").await;

    let after = simple_json(&app, "requests").await;
    let names: Vec<&str> = after["files"]
        .as_array()
        .expect("PEP 691 files array")
        .iter()
        .map(|f| f["filename"].as_str().unwrap_or_default())
        .collect();
    assert!(
        !names.iter().any(|n| n.contains("1.1.0")),
        "blocked version still listed: {names:?}"
    );
    assert_eq!(
        after["versions"],
        serde_json::json!(["1.0.0", "2.0.0b1"]),
        "the PEP 700 summary must agree with the file list"
    );
}

/// The two representations are different bytes for the same URL. Keyed together
/// in the metadata cache, whichever one warmed the entry would be served to
/// clients that asked for the other.
#[actix_web::test]
async fn the_html_and_json_representations_do_not_collide_in_the_cache() {
    let app = app().await;

    // Warm the JSON slot first, then ask for HTML.
    let _ = simple_json(&app, "requests").await;
    let html = simple_html(&app, "requests").await;

    assert!(
        html.trim_start().starts_with("<!DOCTYPE html>"),
        "an HTML request came back as something else: {html}"
    );
}

/// PEP 440 zero-pads for comparison, so `1.1` and `1.1.0` are one version. A
/// block recorded either way must hide the listed spelling.
#[actix_web::test]
async fn proxy_simple_matches_across_pep440_spellings() {
    let app = app().await;

    block_version(&app, "local-pypi", "requests", "1.1").await;

    assert!(
        !simple_html(&app, "requests")
            .await
            .contains("requests-1.1.0"),
        "the short block did not match the three-component listing"
    );
}

#[actix_web::test]
async fn proxy_simple_html_content_type_is_html() {
    let app = app().await;

    let req = TestRequest::get()
        .uri("/proxy/local-pypi/simple/requests/")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .expect("content-type set")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(ct.starts_with("text/html"), "content-type was {ct}");
}

/// File URLs must point back at this proxy, or every download routes around its
/// cache, its audit trail and the download-time gate.
#[actix_web::test]
async fn proxy_simple_html_rewrites_file_urls_to_this_host() {
    let app = app().await;

    let html = simple_html(&app, "requests").await;
    assert!(
        !html.contains("files.invalid"),
        "still pointing upstream: {html}"
    );
    assert!(html.contains("/proxy/local-pypi/packages/requests-1.0.0.tar.gz"));
}

#[actix_web::test]
async fn proxy_another_package_is_untouched() {
    let app = app().await;

    block_version(&app, "local-pypi", "requests", "1.1.0").await;

    assert!(simple_html(&app, "flask").await.contains("flask-1.1.0"));
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-pypi", "requests", "1.1.0").await;

    let req = TestRequest::get()
        .uri("/proxy/local-pypi/packages/requests-1.1.0.tar.gz")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
