//! Blocking a Composer version hides it from p2 metadata — including from the
//! minified encoding Packagist actually serves.
//!
//! This is the one protocol where a naive filter produces a *well-formed
//! document describing the wrong packages*. In `"minified": "composer/2.0"`
//! each entry omits every key identical to the previous entry, so deleting a
//! middle entry silently changes what every entry after it inherits. The filter
//! expands, removes, and re-minifies; the test that matters here is the one
//! that checks an entry after the removed one still means what it meant.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::{Map, Value};

const PKG: &str = "monolog/monolog";

async fn app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-php", "composer", RegistryMode::Proxy, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

async fn p2<S>(app: &S, package: &str) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!("/proxy/local-php/p2/{package}.json"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    read_body_json(call_service(app, req).await).await
}

/// Expand a `composer/2.0` minified list the way a Composer client does, so a
/// test can assert on what an entry *means* rather than on what it spells out.
fn expand(list: &[Value]) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    let mut current = Map::new();
    for entry in list {
        for (k, v) in entry.as_object().expect("an entry object") {
            if v.is_null() {
                current.remove(k);
            } else {
                current.insert(k.clone(), v.clone());
            }
        }
        out.push(current.clone());
    }
    out
}

fn entries_of(doc: &Value, package: &str) -> Vec<Value> {
    doc["packages"][package]
        .as_array()
        .unwrap_or_else(|| panic!("a p2 version list for {package}: {doc}"))
        .clone()
}

fn entries(doc: &Value) -> Vec<Value> {
    entries_of(doc, PKG)
}

fn versions_of(doc: &Value, package: &str) -> Vec<String> {
    expand(&entries_of(doc, package))
        .iter()
        .map(|e| e["version"].as_str().unwrap_or_default().to_owned())
        .collect()
}

fn versions(doc: &Value) -> Vec<String> {
    versions_of(doc, PKG)
}

#[actix_web::test]
async fn proxy_p2_hides_a_blocked_version() {
    let app = app().await;

    assert_eq!(
        versions(&p2(&app, PKG).await),
        ["2.0.0-beta.1", "1.1.0", "1.0.0"]
    );

    block_version(&app, "local-php", PKG, "1.1.0").await;

    assert_eq!(versions(&p2(&app, PKG).await), ["2.0.0-beta.1", "1.0.0"]);
}

/// The regression that catches silent corruption of the minified encoding.
/// `1.0.0` inherits `require` from `1.1.0`; if `1.1.0` is deleted rather than
/// expanded around, `1.0.0` starts claiming it needs PHP 8.1.
#[actix_web::test]
async fn removing_a_middle_entry_does_not_change_what_later_entries_inherit() {
    let app = app().await;

    let before = expand(&entries(&p2(&app, PKG).await));
    let before_1_0_0 = before
        .iter()
        .find(|e| e["version"] == "1.0.0")
        .expect("1.0.0 is listed")
        .clone();
    assert_eq!(
        before_1_0_0["require"]["php"], ">=7.4",
        "the fixture has to actually exercise inheritance"
    );

    block_version(&app, "local-php", PKG, "1.1.0").await;

    let after = expand(&entries(&p2(&app, PKG).await));
    let after_1_0_0 = after
        .iter()
        .find(|e| e["version"] == "1.0.0")
        .expect("1.0.0 survives");
    assert_eq!(
        *after_1_0_0, before_1_0_0,
        "1.0.0 inherited from the removed entry and now describes a different package"
    );
}

#[actix_web::test]
async fn a_filtered_document_is_still_minified() {
    let app = app().await;

    block_version(&app, "local-php", PKG, "1.1.0").await;

    let doc = p2(&app, PKG).await;
    assert_eq!(
        doc["minified"], "composer/2.0",
        "the encoding declaration must match the encoding"
    );
    let list = entries(&doc);
    assert!(
        list[1].as_object().unwrap().len() < list[0].as_object().unwrap().len(),
        "the second entry should still omit what it inherits: {list:?}"
    );
}

/// Dist URLs must point back at this proxy, or every download routes around its
/// cache, its audit trail and the download-time gate.
#[actix_web::test]
async fn proxy_p2_rewrites_dist_urls_to_this_host() {
    let app = app().await;

    let doc = p2(&app, PKG).await;
    let url = expand(&entries(&doc))[0]["dist"]["url"]
        .as_str()
        .unwrap_or_default()
        .to_owned();

    assert!(
        !url.contains("cdn.invalid"),
        "still pointing upstream: {url}"
    );
    assert!(
        url.ends_with("/proxy/local-php/dist/monolog/monolog/2.0.0-beta.1"),
        "dist URL was {url}"
    );
}

#[actix_web::test]
async fn proxy_p2_of_another_package_is_untouched() {
    let app = app().await;

    block_version(&app, "local-php", PKG, "1.1.0").await;

    assert!(
        versions_of(&p2(&app, "symfony/console").await, "symfony/console")
            .contains(&"1.1.0".to_owned())
    );
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-php", PKG, "1.1.0").await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-php/dist/{PKG}/1.1.0"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
