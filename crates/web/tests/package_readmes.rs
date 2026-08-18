//! What gets stored when a package is published, and for which version
//! (RFC 0007 §2.1, §6.4).
//!
//! Asserted against the store rather than against a response body: nothing on
//! the publish response says a README was captured, and the whole point of this
//! half of the RFC is that four code paths already had the text in hand and
//! threw it away. A test that only checked the `200` would have passed before
//! the change too.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use base64::Engine as _;

use batlehub_adapters::in_memory::InMemoryReadmeRepository;
use batlehub_config::schema::RegistryMode;
use batlehub_core::{
    entities::{ReadmeFormat, ReadmeSource},
    ports::ReadmeRepository,
    services::ReadmeService,
};

/// An app whose README store the test can read back.
async fn app_with_readme_store(
    name: &str,
    registry_type: &str,
    mode: RegistryMode,
) -> (
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    Arc<InMemoryReadmeRepository>,
) {
    let repo = InMemoryReadmeRepository::new();
    let svc = Arc::new(ReadmeService::new(
        Arc::clone(&repo) as Arc<dyn ReadmeRepository>
    ));
    let app = build_local_registry_app(
        local_registry_app_parts_with_readme(name, registry_type, mode, None, Some(svc)),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;
    (app, repo)
}

fn npm_publish_payload(name: &str, version: &str, readme: serde_json::Value) -> serde_json::Value {
    let tarball_b64 = base64::engine::general_purpose::STANDARD.encode(b"fake-tarball-content");
    serde_json::json!({
        "name": name,
        "versions": {
            version: {
                "name": name,
                "version": version,
                "readme": readme,
                "dist": { "shasum": "abc123" }
            }
        },
        "_attachments": {
            format!("{name}-{version}.tgz"): {
                "content_type": "application/octet-stream",
                "data": tarball_b64,
                "length": 20
            }
        }
    })
}

async fn publish_npm(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    name: &str,
    payload: serde_json::Value,
) {
    let req = TestRequest::put()
        .uri(&format!("/proxy/local-npm/{name}"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(payload)
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200, "publish should succeed");
}

/// Each published version keeps its own README. This is the whole reason the
/// store is keyed by version: a package-level row would show 2.x's API to
/// somebody reading about 1.x.
#[actix_web::test]
async fn each_published_npm_version_keeps_its_own_readme() {
    let (app, repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;

    publish_npm(
        &app,
        "my-package",
        npm_publish_payload("my-package", "1.0.0", "# the 1.x API".into()),
    )
    .await;
    publish_npm(
        &app,
        "my-package",
        npm_publish_payload("my-package", "2.0.0", "# the 2.x API".into()),
    )
    .await;

    let one = repo
        .get("local-npm", "my-package", "1.0.0")
        .await
        .unwrap()
        .expect("1.0.0 README stored");
    assert_eq!(one.content, "# the 1.x API");
    assert_eq!(one.format, ReadmeFormat::Markdown);
    // Published here, not read from an upstream — an operator asking where a
    // document came from gets a different answer for each.
    assert_eq!(one.source, ReadmeSource::LocalPublish);
    assert!(!one.package_level);

    assert_eq!(
        repo.get("local-npm", "my-package", "2.0.0")
            .await
            .unwrap()
            .unwrap()
            .content,
        "# the 2.x API"
    );
}

/// npm writes a placeholder string rather than omitting the field when the
/// tarball had no README, so a presence check alone would store an error
/// message as documentation.
#[actix_web::test]
async fn npm_publish_stores_nothing_for_an_empty_or_placeholder_readme() {
    let (app, repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;

    for (version, readme) in [
        ("1.0.0", serde_json::json!("ERROR: No README data found!")),
        ("1.0.1", serde_json::json!("   \n  ")),
        ("1.0.2", serde_json::json!(null)),
    ] {
        publish_npm(&app, "quiet", npm_publish_payload("quiet", version, readme)).await;
        assert!(
            repo.get("local-npm", "quiet", version)
                .await
                .unwrap()
                .is_none(),
            "{version} should have stored no README"
        );
    }
}

/// A publish document's root README describes exactly the version being
/// published — there is no older version it could belong to — so it is used
/// when the version object carries none.
#[actix_web::test]
async fn npm_publish_falls_back_to_the_document_root_readme() {
    let (app, repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;

    let mut payload = npm_publish_payload("rooted", "1.0.0", serde_json::Value::Null);
    payload["readme"] = serde_json::json!("# the package README");
    publish_npm(&app, "rooted", payload).await;

    assert_eq!(
        repo.get("local-npm", "rooted", "1.0.0")
            .await
            .unwrap()
            .expect("root README stored")
            .content,
        "# the package README"
    );
}

/// A republish of the same version with different text replaces the row rather
/// than accumulating two answers for one coordinate.
#[actix_web::test]
async fn a_republished_version_replaces_its_readme() {
    let (app, repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;

    publish_npm(
        &app,
        "twice",
        npm_publish_payload("twice", "1.0.0", "# first".into()),
    )
    .await;
    // The publish itself is refused as a duplicate; the README must not be
    // rewritten by a request that stored no artifact.
    let req = TestRequest::put()
        .uri("/proxy/local-npm/twice")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(npm_publish_payload("twice", "1.0.0", "# second".into()))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 409);

    assert_eq!(
        repo.get("local-npm", "twice", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .content,
        "# first",
        "a refused republish must not overwrite the stored README"
    );
    assert_eq!(
        repo.list_versions_with_readme("local-npm", "twice")
            .await
            .unwrap()
            .len(),
        1
    );
}

// ── cargo ─────────────────────────────────────────────────────────────────────

/// `cargo publish`'s wire format: a little-endian u32 length, the JSON
/// metadata, then the same for the `.crate` bytes.
fn cargo_publish_body(meta: serde_json::Value, crate_bytes: &[u8]) -> Vec<u8> {
    let meta_bytes = serde_json::to_vec(&meta).unwrap();
    let mut body = Vec::new();
    body.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(&meta_bytes);
    body.extend_from_slice(&(crate_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(crate_bytes);
    body
}

fn cargo_metadata(name: &str, version: &str, readme: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "vers": version,
        "deps": [],
        "features": {},
        "authors": [],
        "description": "test crate",
        "readme": readme,
        "readme_file": "README.md",
        "cksum": "",
        "yanked": false,
        "links": null,
    })
}

/// `metadata_to_index_entry` narrows the publish metadata to the nine fields
/// the sparse index carries, and drops `readme` — correctly, it is not index
/// data. The text has to be read before that happens, or it is gone.
#[actix_web::test]
async fn a_published_crates_readme_survives_the_index_entry_narrowing() {
    let (app, repo) = app_with_readme_store("local-cargo", "cargo", RegistryMode::Local).await;

    let body = cargo_publish_body(
        cargo_metadata("mylib", "1.0.0", "# mylib\n\nDoes a thing.".into()),
        b"fake-crate-bytes",
    );
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let stored = repo
        .get("local-cargo", "mylib", "1.0.0")
        .await
        .unwrap()
        .expect("README stored");
    assert_eq!(stored.content, "# mylib\n\nDoes a thing.");
    assert_eq!(stored.format, ReadmeFormat::Markdown);
    assert_eq!(stored.source, ReadmeSource::LocalPublish);
}

/// The workspace's own cargo publish fixture sends `"readme": null` — a crate
/// with no README, not one with an empty document.
#[actix_web::test]
async fn a_null_cargo_readme_stores_nothing() {
    let (app, repo) = app_with_readme_store("local-cargo", "cargo", RegistryMode::Local).await;

    let body = cargo_publish_body(
        cargo_metadata("quiet", "1.0.0", serde_json::Value::Null),
        b"fake-crate-bytes",
    );
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    assert!(repo
        .get("local-cargo", "quiet", "1.0.0")
        .await
        .unwrap()
        .is_none());
}

/// `readme_file` names the file the text came from, so its extension declares
/// the markup: an `.rst` README is stored as RST and shown as escaped source
/// rather than parsed as markdown.
#[actix_web::test]
async fn cargos_readme_file_decides_the_markup() {
    let (app, repo) = app_with_readme_store("local-cargo", "cargo", RegistryMode::Local).await;

    let mut meta = cargo_metadata("rst-lib", "1.0.0", "Heading\n=======".into());
    meta["readme_file"] = serde_json::json!("README.rst");
    let req = TestRequest::put()
        .uri("/proxy/local-cargo/api/v1/crates/new")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(cargo_publish_body(meta, b"fake-crate-bytes"))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    assert_eq!(
        repo.get("local-cargo", "rst-lib", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .format,
        ReadmeFormat::Rst
    );
}

/// With no README service wired at all, publishing still works: the capture is
/// an addition to the publish path, never a gate on it.
#[actix_web::test]
async fn publishing_works_with_no_readme_store_at_all() {
    let app = build_local_registry_app(
        local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    publish_npm(
        &app,
        "unstored",
        npm_publish_payload("unstored", "1.0.0", "# text nobody keeps".into()),
    )
    .await;
}

// ── Deletion (RFC 0007 §5.4) ──────────────────────────────────────────────────

/// A README is deleted with its version. The table has no foreign key —
/// a cascade from anything evictable would take the README with the bytes,
/// which §5.4 rules out — so nothing else will do it.
#[actix_web::test]
async fn deleting_a_version_deletes_its_readme_and_only_its_readme() {
    let (app, repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;

    publish_npm(
        &app,
        "doomed",
        npm_publish_payload("doomed", "1.0.0", "# goes".into()),
    )
    .await;
    publish_npm(
        &app,
        "doomed",
        npm_publish_payload("doomed", "2.0.0", "# stays".into()),
    )
    .await;
    publish_npm(
        &app,
        "bystander",
        npm_publish_payload("bystander", "1.0.0", "# untouched".into()),
    )
    .await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/bulk-delete")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "packages": [{ "name": "doomed", "version": "1.0.0" }]
        }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    assert!(repo
        .get("local-npm", "doomed", "1.0.0")
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .get("local-npm", "doomed", "2.0.0")
        .await
        .unwrap()
        .is_some());
    assert!(repo
        .get("local-npm", "bystander", "1.0.0")
        .await
        .unwrap()
        .is_some());
}

// ── The endpoint (RFC 0007 §4.2) ──────────────────────────────────────────────

async fn get_readme(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    uri: &str,
) -> (actix_web::http::StatusCode, serde_json::Value) {
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    let status = resp.status();
    (status, read_body_json(resp).await)
}

/// Two versions with different READMEs each return their own — the assertion
/// the whole per-version key exists for.
#[actix_web::test]
async fn each_version_serves_its_own_readme_rendered() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "1.0.0", "# The 1.x API".into()),
    )
    .await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "2.0.0", "# The 2.x API".into()),
    )
    .await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/docs/readme?version=1.0.0",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["version"], "1.0.0");
    assert_eq!(body["requested_version"], "1.0.0");
    assert_eq!(body["is_fallback"], false);
    assert_eq!(body["format"], "markdown");
    assert_eq!(body["source"], "local-publish");
    assert_eq!(body["stored"], true);
    assert_eq!(body["truncated"], false);
    // Rendered, not echoed: markdown became HTML.
    let html = body["rendered_html"].as_str().unwrap();
    assert!(html.contains("<h1"), "{html}");
    assert!(html.contains("The 1.x API"), "{html}");
    // `format` defaults to html, so the source is not sent.
    assert!(body["source_text"].is_null());

    let (_, two) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/docs/readme?version=2.0.0",
    )
    .await;
    assert!(two["rendered_html"]
        .as_str()
        .unwrap()
        .contains("The 2.x API"));
}

/// `format=source` returns the text unrendered — what the CLI prints, and what
/// an operator checks the rendering against.
#[actix_web::test]
async fn format_source_returns_the_source_and_no_html() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "1.0.0", "# Title\n\ntext".into()),
    )
    .await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/docs/readme?version=1.0.0&format=source",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["source_text"], "# Title\n\ntext");
    assert!(body["rendered_html"].is_null());

    let (_, both) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/docs/readme?version=1.0.0&format=both",
    )
    .await;
    assert!(both["source_text"].is_string());
    assert!(both["rendered_html"].is_string());
}

/// A version with no README of its own is served the newest that has one, and
/// the response says so rather than presenting prose that belongs to different
/// code.
#[actix_web::test]
async fn a_version_without_a_readme_gets_a_labelled_fallback() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "1.4.2", "# 1.4.2".into()),
    )
    .await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "2.0.0-rc1", serde_json::Value::Null),
    )
    .await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/docs/readme?version=2.0.0-rc1",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["version"], "1.4.2");
    assert_eq!(body["requested_version"], "2.0.0-rc1");
    assert_eq!(body["is_fallback"], true);
}

/// A stored README containing `<script>` comes back without it **through the
/// real handler**, not only through the sanitiser's own unit test.
#[actix_web::test]
async fn a_hostile_readme_is_sanitised_on_the_way_out() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "hostile",
        npm_publish_payload(
            "hostile",
            "1.0.0",
            "# ok\n\n<script>alert(document.cookie)</script>\n\n<img src=x onerror=alert(1)>\n\n[go](javascript:alert(1))"
                .into(),
        ),
    )
    .await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/hostile/readme?version=1.0.0&format=both",
    )
    .await;
    assert_eq!(status, 200);
    let html = body["rendered_html"].as_str().unwrap();
    assert!(!html.contains("script"), "{html}");
    assert!(!html.contains("onerror"), "{html}");
    assert!(!html.contains("javascript"), "{html}");
    assert!(html.contains("ok"), "{html}");
    // The *source* is returned verbatim: the store keeps what the package said,
    // and an operator checking the rendering needs to see it.
    assert!(body["source_text"].as_str().unwrap().contains("<script>"));
}

/// A blocked version serves no README — `403` with the same reason the download
/// path returns, so the operator sees that it exists and why it is refused.
#[actix_web::test]
async fn a_blocked_version_is_403_with_its_reason() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "risky",
        npm_publish_payload("risky", "1.0.0", "# risky".into()),
    )
    .await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/packages/block")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "registry": "local-npm", "name": "risky", "version": "1.0.0",
            "reason": "known-malicious"
        }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/risky/readme?version=1.0.0",
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(body["code"], "readme.blocked");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("known-malicious"),
        "{body}"
    );
}

/// A yanked version serves its README normally: a yank withdraws a
/// recommendation, not the documentation, and the version stays downloadable by
/// exact coordinate.
#[actix_web::test]
async fn a_yanked_version_still_serves_its_readme() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "withdrawn",
        npm_publish_payload("withdrawn", "1.0.0", "# still readable".into()),
    )
    .await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/bulk-yank")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(serde_json::json!({
            "packages": [{ "name": "withdrawn", "version": "1.0.0" }]
        }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-npm/withdrawn/readme?version=1.0.0",
    )
    .await;
    assert_eq!(status, 200);
    assert!(body["rendered_html"]
        .as_str()
        .unwrap()
        .contains("still readable"));
}

/// A registry type with no README says so as a statement, with the reason
/// `readme_support()` carries — so the panel can render it as information
/// rather than as an error, and the published support table cannot disagree.
#[actix_web::test]
async fn a_registry_type_with_no_readme_says_which_shape_of_nothing_it_is() {
    let (app, _repo) = app_with_readme_store("local-maven", "maven", RegistryMode::Local).await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-maven/com.example:lib/readme",
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "readme.unsupported-type");
    assert!(
        body["message"].as_str().unwrap().contains("sentence"),
        "{body}"
    );
}

/// The other shape of nothing: the type could carry one and this package has
/// none stored. Distinguished by `code`, because a panel renders one as a
/// statement and the other as a limit that resolves itself.
#[actix_web::test]
async fn a_package_with_no_stored_readme_is_a_different_404() {
    let (app, _repo) = app_with_readme_store("local-cargo", "cargo", RegistryMode::Local).await;

    let (status, body) = get_readme(
        &app,
        "/api/v1/explore/packages/local-cargo/nothing-here/readme",
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["code"], "readme.none-stored");
    // cargo reads its README out of the `.crate`, so the message says when one
    // would arrive rather than implying there is none.
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("first downloaded"),
        "{body}"
    );
}

/// With no version named, the newest that has one answers — and it is not a
/// fallback, because nothing specific was asked for.
#[actix_web::test]
async fn no_version_named_is_not_a_fallback() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "docs",
        npm_publish_payload("docs", "1.0.0", "# only".into()),
    )
    .await;

    let (status, body) = get_readme(&app, "/api/v1/explore/packages/local-npm/docs/readme").await;
    assert_eq!(status, 200);
    assert_eq!(body["version"], "1.0.0");
    assert!(body["requested_version"].is_null());
    assert_eq!(body["is_fallback"], false);
}

// ── The tri-state on the version table ────────────────────────────────────────

/// `available` for a version that has one; `unknown` — never `false` — for one
/// this instance has not read. A boolean cannot carry the difference, and
/// "we have not looked" is not "there is none".
#[actix_web::test]
async fn the_version_table_reports_the_readme_state_per_version() {
    let (app, _repo) = app_with_readme_store("local-npm", "npm", RegistryMode::Local).await;
    publish_npm(
        &app,
        "mixed",
        npm_publish_payload("mixed", "1.0.0", "# documented".into()),
    )
    .await;
    publish_npm(
        &app,
        "mixed",
        npm_publish_payload("mixed", "2.0.0", serde_json::Value::Null),
    )
    .await;

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/local-npm/mixed")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: serde_json::Value = read_body_json(call_service(&app, req).await).await;

    let state_of = |version: &str| -> String {
        body["versions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["version"] == version)
            .unwrap()["readme"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(state_of("1.0.0"), "available");
    // npm reads the tarball too, so a version with nothing stored is *unknown*.
    assert_eq!(state_of("2.0.0"), "unknown");

    // Every version in this table is one this instance holds, so an empty
    // vulnerability list means scanned-and-clear rather than never-scanned.
    for version in body["versions"].as_array().unwrap() {
        assert_eq!(version["vulnerabilities_scanned"], true);
    }
}

/// On a registry type that has no README at all, the state is `none` — a fact
/// about the ecosystem, not a gap in this instance. `unknown` here would tell a
/// reader to wait for something that is never coming.
#[actix_web::test]
async fn a_type_with_no_readme_reports_none_rather_than_unknown() {
    let (app, _repo) = app_with_readme_store("local-maven", "maven", RegistryMode::Local).await;

    const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>mylib</artifactId>
  <version>1.0.0</version>
  <packaging>jar</packaging>
</project>"#;
    let req = TestRequest::put()
        .uri("/proxy/local-maven/maven2/com/example/mylib/1.0.0/mylib-1.0.0.pom")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/xml"))
        .set_payload(POM)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/local-maven/com.example:mylib")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: serde_json::Value = read_body_json(call_service(&app, req).await).await;
    let versions = body["versions"].as_array().unwrap();
    assert!(
        !versions.is_empty(),
        "expected the published version: {body}"
    );
    assert_eq!(versions[0]["readme"], "none");
}
