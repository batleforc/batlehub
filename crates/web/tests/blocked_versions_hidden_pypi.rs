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

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

const JSON_ACCEPT: &str = "application/vnd.pypi.simple.v1+json";

async fn app() -> impl TestService {
    proxy_registry_app("local-pypi", "pypi").await
}

async fn simple_html<S: TestService>(app: &S, package: &str) -> String {
    get_text(app, &format!("/proxy/local-pypi/simple/{package}/")).await
}

/// The same page under content negotiation — PEP 691's JSON rendering, which
/// pip prefers and which is a different code path through the filter.
async fn simple_json<S: TestService>(app: &S, package: &str) -> Value {
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

    assert_content_type(&app, "/proxy/local-pypi/simple/requests/", "text/html").await;
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

    let req = admin_get("/proxy/local-pypi/packages/requests-1.1.0.tar.gz");
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── PEP 658 metadata siblings (RFC 0009 §12.7) ───────────────────────────────
//
// The simple page BatleHub serves carries upstream's `data-core-metadata`
// attribute — the href rewrite only touches `href`. pip then requests
// `{file}.metadata`, and on a `404` it **fails the install** rather than
// falling back to downloading the wheel. Measured against pip 26.1.2.
//
// So the sibling has to resolve. It is not a distribution of its own: the
// coordinate comes from the stripped filename, and the full name stays the
// artifact sub-coordinate so the two are cached apart.

#[actix_web::test]
async fn a_metadata_sibling_resolves_to_its_distributions_coordinate() {
    let app = app().await;

    let wheel = "/proxy/local-pypi/packages/requests-1.0.0-py3-none-any.whl";
    let sibling = format!("{wheel}.metadata");

    let mut statuses = Vec::new();
    for uri in [wheel, sibling.as_str()] {
        statuses.push(call_service(&app, admin_get(uri)).await.status());
    }

    // Whatever the distribution itself answers, the sibling must answer the
    // same way — in particular it must never be a 422 "cannot parse PyPI
    // filename", which is what rejected it before.
    assert_ne!(
        statuses[1].as_u16(),
        422,
        "the .metadata sibling was rejected as an unparseable filename; pip \
         fails the install on that rather than falling back"
    );
    assert_eq!(
        statuses[0], statuses[1],
        "the sibling should follow its distribution's fate, not diverge from it"
    );
}
