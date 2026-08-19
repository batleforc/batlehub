//! The console's document, served with a policy narrowed to the config
//! (RFC 0013 §7, §11 O5).
//!
//! The unit tests next to `narrow_csp` prove the string transformation. These
//! prove the thing that actually ships: that the document a browser receives
//! carries the narrowed policy, that it does so by either of the two URLs it can
//! be asked for, and that the answer follows a **hot** config change rather than
//! whatever was true at boot.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body, TestRequest};
use actix_web::{web, App};

use batlehub_core::services::{FeatureFlags, LocalRegistryService};

/// The policy as `ui/build/csp.ts` emits it.
const BUILT_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: https://badge.socket.dev; \
     font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; \
     form-action 'self'";

fn document() -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{BUILT_POLICY}\" />\n\
         <title>BatleHub</title>\n</head>\n<body><div id=\"app\"></div></body>\n</html>\n"
    )
}

/// A static dir holding one `index.html`, plus an app serving it the way
/// `server_factory` does — the SPA routes first, the file service behind them.
async fn app_with(
    local_svc: Arc<LocalRegistryService>,
) -> (
    tempfile::TempDir,
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
) {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("index.html"), document()).expect("write index.html");
    let path = dir.path().to_path_buf();

    let app = actix_web::test::init_service(
        App::new()
            .app_data(web::Data::new(batlehub_web::SpaDir(path.clone())))
            .app_data(web::Data::new(local_svc))
            .configure(batlehub_web::configure_spa)
            .service(
                actix_files::Files::new("/", &path)
                    .index_file("index.html")
                    .use_last_modified(true),
            ),
    )
    .await;
    (dir, app)
}

/// `make_local_svc` with one registry's badge flag set.
async fn svc_with_badge(on: bool) -> Arc<LocalRegistryService> {
    let storage = batlehub_adapters::in_memory::InMemoryStorageBackend::new();
    let svc = make_local_svc(storage);
    svc.hot
        .write()
        .await
        .feature_flags
        .insert("npm".to_owned(), FeatureFlags { socket_badge: on });
    svc
}

async fn body_of(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    uri: &str,
) -> String {
    let resp = call_service(app, TestRequest::get().uri(uri).to_request()).await;
    assert_eq!(resp.status(), 200, "GET {uri}");
    String::from_utf8(read_body(resp).await.to_vec()).expect("utf-8")
}

#[actix_web::test]
async fn the_badge_origin_is_dropped_when_no_registry_draws_one() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    let html = body_of(&app, "/").await;

    assert!(
        !html.contains("badge.socket.dev"),
        "the document still admits an origin it will never call: {html}"
    );
    assert!(html.contains("img-src 'self' data:"), "{html}");
    // Everything else is the built policy, byte for byte.
    assert!(html.contains("script-src 'self'"), "{html}");
    assert!(html.contains("form-action 'self'"), "{html}");
}

#[actix_web::test]
async fn the_badge_origin_stays_when_a_registry_draws_one() {
    let (_dir, app) = app_with(svc_with_badge(true).await).await;

    let html = body_of(&app, "/").await;

    assert!(html.contains("https://badge.socket.dev"), "{html}");
}

/// `Files` is mounted at `/` and would serve this straight off disk. Which
/// policy a reader gets must not depend on the URL they arrived by.
#[actix_web::test]
async fn the_same_narrowing_applies_to_the_document_by_name() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    let by_name = body_of(&app, "/index.html").await;

    assert!(!by_name.contains("badge.socket.dev"), "{by_name}");
    assert_eq!(by_name, body_of(&app, "/").await);
}

/// The point of doing this at serve time rather than at build time: an operator
/// turning the flag off does not have to rebuild the console.
#[actix_web::test]
async fn the_policy_follows_a_hot_config_change() {
    let svc = svc_with_badge(true).await;
    let (_dir, app) = app_with(Arc::clone(&svc)).await;
    assert!(body_of(&app, "/").await.contains("badge.socket.dev"));

    svc.hot.write().await.feature_flags.insert(
        "npm".to_owned(),
        FeatureFlags {
            socket_badge: false,
        },
    );

    assert!(
        !body_of(&app, "/").await.contains("badge.socket.dev"),
        "the document was still describing the old config"
    );
}

/// An absent entry means the badge is *on* (`FeatureFlags::default()`), which is
/// how `explore/detail.rs` reads it when deciding to emit the URL. A policy
/// narrower than the page's own behaviour is a broken image in every row.
#[actix_web::test]
async fn a_registry_with_no_flags_block_still_gets_its_origin() {
    let storage = batlehub_adapters::in_memory::InMemoryStorageBackend::new();
    let svc = make_local_svc(storage);
    svc.hot
        .write()
        .await
        .feature_flags
        .insert("npm".to_owned(), FeatureFlags::default());
    let (_dir, app) = app_with(svc).await;

    assert!(body_of(&app, "/").await.contains("badge.socket.dev"));
}

/// The assets keep coming from the file service, untouched — the narrowing is
/// for the document and nothing else.
#[actix_web::test]
async fn an_asset_is_still_served_by_the_file_service() {
    let (dir, app) = app_with(svc_with_badge(false).await).await;
    std::fs::write(dir.path().join("app.js"), "console.log(1)\n").expect("write asset");

    let js = body_of(&app, "/app.js").await;

    assert_eq!(js, "console.log(1)\n");
}
