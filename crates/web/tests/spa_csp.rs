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
            .app_data(web::Data::new(local_svc))
            .configure(|cfg| batlehub_web::configure_spa(cfg, path.clone())),
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

// ── Deep links ────────────────────────────────────────────────────────────────
//
// `is_console_route` is unit-tested exhaustively next to the code. These are
// about the wiring: that the fallback is actually reached, that it carries the
// same narrowed policy as the front door, and that the paths it must not answer
// still fail through the real service stack rather than only in a pure function.

#[actix_web::test]
async fn a_deep_link_serves_the_console_with_its_narrowed_policy() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    let html = body_of(&app, "/packages/npm/chalk?version=4.0.2&q=4.0&page=2").await;

    assert!(html.contains("<div id=\"app\">"), "{html}");
    // The whole point of the fallback: the same document, so the same policy.
    assert!(!html.contains("badge.socket.dev"), "{html}");
}

#[actix_web::test]
async fn a_missing_asset_is_still_a_404() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    let resp = call_service(
        &app,
        TestRequest::get()
            .uri("/assets/index-deadbeef.js")
            .to_request(),
    )
    .await;

    assert_eq!(resp.status(), 404);
}

/// The failure that would matter: a package manager asking for an artifact this
/// instance does not have must be told so.
#[actix_web::test]
async fn an_unknown_api_path_is_not_answered_with_the_console() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    for uri in ["/api/v1/nope", "/proxy/npm1/chalk/-/chalk-9.9.9.tgz"] {
        let resp = call_service(&app, TestRequest::get().uri(uri).to_request()).await;
        assert_eq!(resp.status(), 404, "{uri}");
        let body = String::from_utf8(read_body(resp).await.to_vec()).unwrap();
        assert!(!body.contains("<div id=\"app\">"), "{uri} got the console");
    }
}

/// A `POST` to a path that does not exist is a mistake, and a page would hide
/// it.
///
/// The answer is `405` rather than `404` because `actix_files` refuses the
/// method before it ever consults the fallback — which is the point: the
/// fallback's own `GET`-only guard is a second lock on a door the file service
/// already holds shut. What matters is that neither of them answers with a page.
#[actix_web::test]
async fn a_post_to_an_unknown_path_is_not_the_console() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;

    let resp = call_service(
        &app,
        TestRequest::post().uri("/packages/npm/chalk").to_request(),
    )
    .await;

    assert_eq!(resp.status(), 405);
    let body = String::from_utf8(read_body(resp).await.to_vec()).unwrap();
    assert!(!body.contains("<div id=\"app\">"), "a POST got the console");
}

/// A file that *is* on disk still comes from the file service — the fallback
/// only ever sees what `Files` could not resolve.
#[actix_web::test]
async fn an_existing_asset_still_wins_over_the_fallback() {
    let (dir, app) = app_with(svc_with_badge(false).await).await;
    std::fs::create_dir_all(dir.path().join("assets")).unwrap();
    std::fs::write(dir.path().join("assets/app.js"), "export const a = 1\n").unwrap();

    assert_eq!(
        body_of(&app, "/assets/app.js").await,
        "export const a = 1\n"
    );
}

/// The document never comes off disk, whatever it is spelled like.
///
/// `PathBufWrap::parse_path` skips empty segments, so `//index.html` and
/// `/index.html/` matched neither narrow route, resolved to the same file, and
/// were served with the **built** policy — still admitting the badge origin on
/// an instance where no registry draws one.
#[actix_web::test]
async fn the_index_is_narrowed_by_every_spelling_that_reaches_it() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;
    // The two canonical spellings answer the document, narrowed.
    for uri in ["/", "/index.html"] {
        let html = body_of(&app, uri).await;
        assert!(
            !html.contains("badge.socket.dev"),
            "GET {uri} served the un-narrowed policy: {html}"
        );
    }
    // The odd ones no longer answer it off disk at all. `404` is a narrowing and
    // therefore allowed; serving the built policy was not.
    for uri in ["//index.html", "/index.html/"] {
        let resp = call_service(&app, TestRequest::get().uri(uri).to_request()).await;
        let status = resp.status();
        let body = String::from_utf8(read_body(resp).await.to_vec()).expect("utf-8");
        assert!(
            !body.contains("badge.socket.dev"),
            "GET {uri} → {status} served the un-narrowed policy: {body}"
        );
    }
}

/// A percent-encoded reserved prefix is still a reserved prefix.
///
/// Routing happens against the requoted path, so `/%61pi/v1/nope` matches no API
/// route and reaches the fallback — where the raw spelling does not start with
/// `/api`. It was answered `200 text/html`: an API `404` dressed as the console,
/// and, under `/%70roxy/…`, markup where a package manager expected an artifact.
#[actix_web::test]
async fn an_encoded_reserved_prefix_is_not_the_console() {
    let (_dir, app) = app_with(svc_with_badge(false).await).await;
    for uri in ["/%61pi/v1/nope", "/%70roxy/npm/lodash", "/%2561pi/v1/nope"] {
        let resp = call_service(&app, TestRequest::get().uri(uri).to_request()).await;
        assert_eq!(resp.status(), 404, "GET {uri} was answered with a page");
    }
    // …and an ordinary deep link still is one.
    let html = body_of(&app, "/packages/npm/lodash.merge").await;
    assert!(html.contains("<div id=\"app\">"), "{html}");
}
