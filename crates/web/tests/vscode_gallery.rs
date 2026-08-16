//! BatleHub as an editor's extension marketplace.
//!
//! Before this existed the whole client surface was the raw VSIX download, so
//! an editor could not be pointed at BatleHub at all. These tests pin the two
//! client protocols that changed that — the VS Code gallery
//! (`extensionquery`, assets, `item`) and the OpenVSX REST API — and the
//! property that makes either of them worth having: **every URL in a response
//! points back at this proxy**, so downloads stay inside the cache, the audit
//! trail and the policy gates.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use std::io::Write;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::{json, Value};

const EXT: &str = "acme.tool";
const VERSION: &str = "1.2.3";

/// A real VSIX: a ZIP with `extension/package.json` and the prose files an
/// editor's detail pane asks for. Built rather than faked because the asset
/// routes serve files *out of* it, so a placeholder would test nothing.
fn make_vsix() -> Vec<u8> {
    let manifest = json!({
        "publisher": "acme",
        "name": "tool",
        "version": VERSION,
        "displayName": "Acme Tool",
        "description": "Does the thing",
        "categories": ["Linters"],
        "keywords": ["acme"],
        "icon": "icon.png",
        "engines": { "vscode": "^1.85.0" }
    })
    .to_string();

    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in [
            ("extension/package.json", manifest.as_bytes()),
            ("extension/README.md", b"# Acme Tool" as &[u8]),
            ("extension/CHANGELOG.md", b"## 1.2.3" as &[u8]),
            ("extension/LICENSE.txt", b"MIT" as &[u8]),
            ("extension/icon.png", b"\x89PNG\r\n\x1a\n" as &[u8]),
        ] {
            w.start_file(name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }
    buf
}

async fn gallery_app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let app = build_local_registry_app(
        local_registry_app_parts("local-vsx", "openvsx", RegistryMode::Local, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    let req = TestRequest::put()
        .uri(&format!("/proxy/local-vsx/{EXT}/{VERSION}/vsix"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/octet-stream"))
        .set_payload(make_vsix())
        .to_request();
    let resp = call_service(&app, req).await;
    assert!(resp.status().is_success(), "publish: {}", resp.status());
    app
}

async fn query<S>(app: &S, body: Value) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::post()
        .uri("/proxy/local-vsx/vscode/gallery/extensionquery")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(body)
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200, "extensionquery should answer");
    read_body_json(resp).await
}

/// The body VS Code sends to resolve one extension — install and update both
/// take this path.
fn lookup_body(name: &str) -> Value {
    json!({
        "filters": [{
            "criteria": [
                { "filterType": 8, "value": "Microsoft.VisualStudio.Code" },
                { "filterType": 7, "value": name }
            ],
            "pageNumber": 1, "pageSize": 50, "sortBy": 0, "sortOrder": 0
        }],
        "assetTypes": [],
        "flags": 0x1 | 0x2 | 0x4 | 0x10 | 0x80 | 0x100
    })
}

fn first_extension(doc: &Value) -> &Value {
    &doc["results"][0]["extensions"][0]
}

fn total_count(doc: &Value) -> u64 {
    doc["results"][0]["resultMetadata"][0]["metadataItems"][0]["count"]
        .as_u64()
        .expect("TotalCount")
}

// ── the gallery ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn an_exact_lookup_returns_the_extension_with_a_total_count() {
    let app = gallery_app().await;
    let doc = query(&app, lookup_body(EXT)).await;

    assert_eq!(total_count(&doc), 1, "the editor pages against this");
    let e = first_extension(&doc);
    assert_eq!(e["publisher"]["publisherName"], "acme");
    assert_eq!(e["extensionName"], "tool");
    assert_eq!(e["displayName"], "Acme Tool");
    assert_eq!(e["versions"][0]["version"], VERSION);
}

#[actix_web::test]
async fn an_unknown_extension_is_an_empty_result_not_an_error() {
    let app = gallery_app().await;
    let doc = query(&app, lookup_body("nobody.nothing")).await;

    assert_eq!(total_count(&doc), 0);
    assert_eq!(doc["results"][0]["extensions"], json!([]));
}

/// The property the whole feature rests on: served with upstream URLs, every
/// download would route around the cache, the audit trail and the download
/// gate.
#[actix_web::test]
async fn every_url_in_the_response_points_at_this_proxy() {
    let app = gallery_app().await;
    let doc = query(&app, lookup_body(EXT)).await;
    let v = &first_extension(&doc)["versions"][0];

    let asset_uri = v["assetUri"].as_str().expect("assetUri");
    assert!(
        asset_uri.ends_with("/proxy/local-vsx/vscode/asset/acme/tool/1.2.3"),
        "assetUri was {asset_uri}"
    );
    assert_eq!(v["assetUri"], v["fallbackAssetUri"]);

    let rendered = serde_json::to_string(&doc).unwrap();
    assert!(
        !rendered.contains("marketplace.visualstudio.com") && !rendered.contains("open-vsx.org"),
        "an upstream URL leaked into the response: {rendered}"
    );
}

#[actix_web::test]
async fn the_engine_range_is_reported_so_the_editor_can_judge_compatibility() {
    let app = gallery_app().await;
    let doc = query(&app, lookup_body(EXT)).await;

    let props = first_extension(&doc)["versions"][0]["properties"]
        .as_array()
        .expect("properties");
    let engine = props
        .iter()
        .find(|p| p["key"] == "Microsoft.VisualStudio.Code.Engine")
        .expect("the editor refuses a version whose engine it cannot read");
    assert_eq!(engine["value"], "^1.85.0");
}

/// A query for a different editor, or for a curated list this registry does not
/// keep, answers with nothing rather than with the whole catalogue.
#[actix_web::test]
async fn an_unanswerable_query_returns_nothing_rather_than_everything() {
    let app = gallery_app().await;

    for criteria in [
        json!([{ "filterType": 8, "value": "Microsoft.VisualStudio.IDE" }]),
        json!([{ "filterType": 9, "value": "" }]),
    ] {
        let doc = query(
            &app,
            json!({ "filters": [{ "criteria": criteria }], "flags": 0x1 }),
        )
        .await;
        assert_eq!(total_count(&doc), 0, "criteria {criteria} should be empty");
    }
}

#[actix_web::test]
async fn free_text_search_finds_the_extension() {
    let app = gallery_app().await;
    let doc = query(
        &app,
        json!({
            "filters": [{ "criteria": [{ "filterType": 10, "value": "acme" }] }],
            "flags": 0x1
        }),
    )
    .await;

    assert_eq!(total_count(&doc), 1);
    assert_eq!(first_extension(&doc)["extensionName"], "tool");
}

// ── assets ───────────────────────────────────────────────────────────────────

async fn asset<S>(app: &S, asset_type: &str) -> (u16, String, Vec<u8>)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/vscode/asset/acme/tool/{VERSION}/{asset_type}"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    let status = resp.status().as_u16();
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, ct, read_body(resp).await.to_vec())
}

#[actix_web::test]
async fn every_advertised_asset_type_serves_its_bytes() {
    let app = gallery_app().await;

    let (status, ct, body) = asset(&app, "Microsoft.VisualStudio.Code.Manifest").await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("application/json"), "manifest ct was {ct}");
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["name"],
        "tool"
    );

    let (status, ct, body) = asset(&app, "Microsoft.VisualStudio.Services.Content.Details").await;
    assert_eq!(status, 200);
    assert!(ct.starts_with("text/markdown"), "README ct was {ct}");
    assert_eq!(body, b"# Acme Tool");

    let (status, _, body) = asset(&app, "Microsoft.VisualStudio.Services.Content.Changelog").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"## 1.2.3");

    let (status, _, body) = asset(&app, "Microsoft.VisualStudio.Services.Content.License").await;
    assert_eq!(status, 200);
    assert_eq!(body, b"MIT");

    let (status, ct, _) = asset(&app, "Microsoft.VisualStudio.Services.Icons.Default").await;
    assert_eq!(status, 200);
    assert_eq!(ct, "image/png");

    let (status, _, body) = asset(&app, "Microsoft.VisualStudio.Services.VSIXPackage").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..2], b"PK", "the package itself is the archive");
}

/// An extension that ships no changelog is a `404` for that asset, not a
/// failure of the whole listing.
#[actix_web::test]
async fn an_asset_type_the_extension_does_not_ship_is_a_404() {
    let app = build_local_registry_app(
        local_registry_app_parts("local-vsx", "openvsx", RegistryMode::Local, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    // Published with a manifest and nothing else.
    let mut buf = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        w.start_file("extension/package.json", opts).unwrap();
        w.write_all(br#"{"publisher":"acme","name":"tool","version":"1.2.3"}"#)
            .unwrap();
        w.finish().unwrap();
    }
    let req = TestRequest::put()
        .uri(&format!("/proxy/local-vsx/{EXT}/{VERSION}/vsix"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_payload(buf)
        .to_request();
    assert!(call_service(&app, req).await.status().is_success());

    let (status, _, _) = asset(&app, "Microsoft.VisualStudio.Services.Content.Changelog").await;
    assert_eq!(status, 404);
}

#[actix_web::test]
async fn vspackage_serves_the_package() {
    let app = gallery_app().await;

    let req = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/vscode/gallery/publishers/acme/vsextensions/tool/{VERSION}/vspackage"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    assert_eq!(&read_body(resp).await[..2], b"PK");
}

#[actix_web::test]
async fn unpkg_serves_a_file_and_rejects_traversal() {
    let app = gallery_app().await;

    let ok = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/vscode/unpkg/acme/tool/{VERSION}/README.md"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, ok).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(read_body(resp).await, b"# Acme Tool".as_slice());

    let evil = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/vscode/unpkg/acme/tool/{VERSION}/../../../etc/passwd"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_ne!(
        call_service(&app, evil).await.status(),
        200,
        "a traversal must never succeed"
    );
}

#[actix_web::test]
async fn item_redirects_to_the_console_page() {
    let app = gallery_app().await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-vsx/vscode/item?itemName={EXT}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;

    assert_eq!(resp.status(), 302);
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        location.contains("/packages/local-vsx/"),
        "location was {location}"
    );
}

/// Route-ordering guard. `vscode/item` is two segments, exactly the shape of
/// the shared npm `{name}/{version}` wildcard, so it is only reachable while
/// the gallery routes stay registered ahead of it.
#[actix_web::test]
async fn the_gallery_routes_are_not_swallowed_by_the_npm_wildcards() {
    let app = gallery_app().await;

    let req = TestRequest::get()
        .uri(&format!("/proxy/local-vsx/vscode/item?itemName={EXT}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;

    assert_eq!(
        resp.status(),
        302,
        "a 200 here means the npm version route answered instead"
    );
}

// ── the OpenVSX API ──────────────────────────────────────────────────────────

async fn api_get<S>(app: &S, path: &str) -> (u16, Value)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(path)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    let status = resp.status().as_u16();
    (status, read_body_json(resp).await)
}

#[actix_web::test]
async fn the_openvsx_api_describes_the_extension() {
    let app = gallery_app().await;

    let (status, doc) = api_get(&app, "/proxy/local-vsx/api/acme/tool").await;
    assert_eq!(status, 200);
    assert_eq!(doc["namespace"], "acme");
    assert_eq!(doc["name"], "tool");
    assert_eq!(doc["version"], VERSION);
    assert_eq!(doc["displayName"], "Acme Tool");

    let download = doc["files"]["download"].as_str().unwrap_or_default();
    assert!(
        download.contains("/proxy/local-vsx/vscode/asset/acme/tool/"),
        "download URL was {download}"
    );
}

#[actix_web::test]
async fn the_openvsx_api_serves_a_pinned_version_and_404s_an_absent_one() {
    let app = gallery_app().await;

    let (status, doc) = api_get(&app, &format!("/proxy/local-vsx/api/acme/tool/{VERSION}")).await;
    assert_eq!(status, 200);
    assert_eq!(doc["version"], VERSION);

    let (status, _) = api_get(&app, "/proxy/local-vsx/api/acme/tool/9.9.9").await;
    assert_eq!(status, 404);
}

/// `-` must not be read as a publisher name, which is only true while
/// `api/-/search` is registered ahead of `api/{namespace}/{extension}`.
#[actix_web::test]
async fn openvsx_search_is_not_taken_for_a_namespace() {
    let app = gallery_app().await;

    let (status, doc) = api_get(&app, "/proxy/local-vsx/api/-/search?query=acme").await;
    assert_eq!(status, 200);
    assert_eq!(doc["totalSize"], 1);
    assert_eq!(doc["extensions"][0]["name"], "tool");
    assert_eq!(doc["extensions"][0]["namespace"], "acme");
}

#[actix_web::test]
async fn the_openvsx_api_serves_files_out_of_the_extension() {
    let app = gallery_app().await;

    let req = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/api/acme/tool/{VERSION}/file/acme.tool-{VERSION}.vsix"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(&read_body(resp).await[..2], b"PK");

    let req = TestRequest::get()
        .uri(&format!(
            "/proxy/local-vsx/api/acme/tool/{VERSION}/file/README.md"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(read_body(resp).await, b"# Acme Tool".as_slice());
}

/// The gallery is a read surface like any other: an identity the registry does
/// not admit gets a refusal, not a quietly empty catalogue.
#[actix_web::test]
async fn an_unauthorised_identity_is_refused_rather_than_shown_nothing() {
    let app = gallery_app().await;

    let req = TestRequest::get()
        .uri("/proxy/local-vsx/api/acme/tool")
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status(),
        403,
        "anonymous has releases:read but not source:read on this registry"
    );
}

/// A lookup by `extensionId` (filterType 4) resolves too. The id is a uuid
/// derived from the name, so it cannot take the fast name-lookup path — this
/// pins that it still finds the extension rather than answering with nothing.
#[actix_web::test]
async fn a_lookup_by_extension_id_resolves() {
    let app = gallery_app().await;

    let by_name = query(&app, lookup_body(EXT)).await;
    let id = first_extension(&by_name)["extensionId"]
        .as_str()
        .expect("extensionId")
        .to_owned();

    let by_id = query(
        &app,
        json!({
            "filters": [{ "criteria": [{ "filterType": 4, "value": id }] }],
            "flags": 0x1
        }),
    )
    .await;

    assert_eq!(total_count(&by_id), 1);
    assert_eq!(first_extension(&by_id)["extensionName"], "tool");
}
