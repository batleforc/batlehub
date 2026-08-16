//! Blocking a crate version marks it `yanked` in the sparse index — and the
//! sparse index route is now authorised.
//!
//! Two changes meet here. Cargo is the one protocol where a blocked version is
//! **marked rather than removed**: `yanked` is cargo's own "exists, do not
//! select", so resolution skips it while an existing `Cargo.lock` that already
//! pins it still resolves and then meets the download gate — which is where
//! that conversation belongs. Deleting the line makes cargo report the crate as
//! never having had the version, which breaks lockfile diagnostics for no gain.
//!
//! And the route itself moved behind `ProxyService`. It used to answer with a
//! bare upstream GET: no rule chain, no access event, no cache. The
//! authorisation gap was the more serious of the two findings, and is closed by
//! the same change.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body, TestRequest};
use batlehub_config::schema::RegistryMode;
use batlehub_core::{entities::Role, rules::RbacRule, services::RegistryPolicy};
use serde_json::Value;

/// `serde` sits at the four-plus-character layout, so its index path exercises
/// the two-prefix form rather than the short-name special cases.
const INDEX_PATH: &str = "se/rd/serde";

async fn app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-crates", "cargo", RegistryMode::Proxy, None),
        // The route 404s before authorising anything unless a sparse index is
        // configured for the registry.
        cargo_index_map("local-crates"),
        None,
    )
    .await
}

async fn index_entry<S>(app: &S, path: &str) -> String
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!("/proxy/local-crates/registry/{path}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body = read_body(call_service(app, req).await).await;
    String::from_utf8(body.to_vec()).expect("the sparse index is UTF-8")
}

fn line(body: &str, n: usize) -> Value {
    serde_json::from_str(body.lines().nth(n).expect("index line")).expect("index line is JSON")
}

/// The behaviour that separates cargo from every other protocol here.
#[actix_web::test]
async fn proxy_index_marks_a_blocked_version_yanked_rather_than_removing_it() {
    let app = app().await;

    let before = index_entry(&app, INDEX_PATH).await;
    assert_eq!(before.lines().count(), 2);
    assert_eq!(line(&before, 1)["yanked"], Value::Bool(false));

    block_version(&app, "local-crates", "serde", "1.1.0").await;

    let after = index_entry(&app, INDEX_PATH).await;
    assert_eq!(
        after.lines().count(),
        2,
        "the line must stay, or cargo reports the version as never having existed"
    );
    assert_eq!(line(&after, 1)["yanked"], Value::Bool(true));
    assert_eq!(line(&after, 0)["yanked"], Value::Bool(false));
}

/// The checksum is what cargo verifies a downloaded `.crate` against; a filter
/// that dropped it would break every unblocked version in the index.
#[actix_web::test]
async fn proxy_index_preserves_every_other_field() {
    let app = app().await;

    block_version(&app, "local-crates", "serde", "1.1.0").await;

    let l = line(&index_entry(&app, INDEX_PATH).await, 1);
    assert_eq!(l["name"], "serde");
    assert_eq!(l["vers"], "1.1.0");
    assert_eq!(l["cksum"], "bbb");
}

#[actix_web::test]
async fn proxy_index_content_type_is_plain_text() {
    let app = app().await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-crates/registry/{INDEX_PATH}"))
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
    assert!(ct.starts_with("text/plain"), "content-type was {ct}");
}

/// The finding from RFC 0006 §6.7, in its own right: this route used to answer
/// without consulting the rule chain at all, so a private cargo registry's
/// crate names and versions were readable by anyone who could reach the port.
#[actix_web::test]
async fn the_sparse_index_refuses_an_identity_without_read_access() {
    let p = local_registry_app_parts("local-crates", "cargo", RegistryMode::Proxy, None);
    p.proxy_svc.hot.write().await.policies.insert(
        "local-crates".to_owned(),
        Arc::new(anonymous_denied_policy()),
    );
    let app = build_local_registry_app(p, cargo_index_map("local-crates"), None).await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-crates/registry/{INDEX_PATH}"))
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status(),
        403,
        "anonymous must not be able to enumerate a private registry's crates"
    );
}

/// Hiding governs resolution, not diagnosis: a lockfile that already pins the
/// blocked version resolves — and then gets the operator's `403` and reason.
#[actix_web::test]
async fn proxy_direct_download_of_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-crates", "serde", "1.1.0").await;

    let req = TestRequest::get()
        .uri("/proxy/local-crates/serde/1.1.0/download")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

/// A registry policy that grants anonymous callers nothing.
fn anonymous_denied_policy() -> RegistryPolicy {
    let perms = std::collections::HashMap::from([
        (Role::Anonymous, Vec::new()),
        (Role::Admin, vec!["*".to_owned()]),
    ]);
    RegistryPolicy {
        metadata_ttl: None,
        firewall_only: false,
        serve_stale_metadata: false,
        artifact_ttl: None,
        rules: vec![Box::new(RbacRule::new(perms))],
    }
}
