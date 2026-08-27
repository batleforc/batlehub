//! RFC 0016 — a published version coordinate is never occupied twice.
//!
//! Two things are asserted here and nowhere else. First, that deleting a version
//! **spends** its number: the bytes go, the listing entry goes, and a later
//! publish onto the same coordinate is refused for good. Second, that the
//! `deleted_at IS NULL` predicate actually reached every ecosystem's listing —
//! asserted per registry type rather than per query, because "we added it to the
//! shared helper" is exactly the reasoning the 2026-08-26 survey found to be
//! false eight times.
//!
//! The per-ecosystem test publishes through `LocalRegistryService` rather than
//! through each protocol's own publish endpoint. That is deliberate: what is
//! under test is the *read* path, and routing a dozen wire formats through their
//! publish handlers would make the table a test of the publishers instead.

mod common;
#[allow(unused_imports)]
use common::*;

use std::time::Duration;

use actix_web::test::{call_service, read_body, read_body_json, TestRequest};
use serde_json::{json, Value};

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{Identity, Role};
use batlehub_core::services::{artifact_storage_key, PublishRequest, RetentionPolicy};

fn publisher() -> Identity {
    Identity {
        user_id: Some("user-1".to_owned()),
        role: Role::User,
        auth_provider: None,
        groups: vec![],
    }
}

/// Publish one version straight through the service, with `index_metadata` the
/// caller chooses. Returns the artifact bytes' storage key.
async fn publish(
    local_svc: &batlehub_core::services::LocalRegistryService,
    registry: &str,
    name: &str,
    version: &str,
    metadata: Value,
) -> String {
    let artifact = bytes::Bytes::from_static(b"artifact-bytes");
    let checksum = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&artifact));
    local_svc
        .publish(PublishRequest {
            registry: registry.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            artifact,
            checksum,
            index_metadata: metadata,
            unlisted: false,
            publisher: publisher(),
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect("publish");
    artifact_storage_key(registry, name, version)
}

fn bulk_delete_request(registry: &str, name: &str, version: &str) -> actix_http::Request {
    TestRequest::post()
        .uri(&format!("/api/v1/admin/registries/{registry}/bulk-delete"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(json!({ "packages": [{ "name": name, "version": version }] }))
        .to_request()
}

// ── The coordinate is spent ───────────────────────────────────────────────────

/// The headline invariant: a deleted version cannot be published again, and the
/// refusal names the reason rather than claiming the version already exists.
#[actix_web::test]
async fn a_deleted_coordinate_cannot_be_republished() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(&local_svc, "local-npm", "my-pkg", "1.4.0", json!({})).await;

    let resp = call_service(&app, bulk_delete_request("local-npm", "my-pkg", "1.4.0")).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["succeeded"], 1, "{body}");

    let err = local_svc
        .publish(PublishRequest {
            registry: "local-npm".to_owned(),
            name: "my-pkg".to_owned(),
            version: "1.4.0".to_owned(),
            artifact: bytes::Bytes::from_static(b"entirely-different-bytes"),
            checksum: "deadbeef".to_owned(),
            index_metadata: json!({}),
            unlisted: false,
            publisher: publisher(),
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect_err("a spent coordinate must refuse a re-publish");

    let message = err.to_string();
    assert!(
        message.contains("never reused") && message.contains("1.4.0"),
        "the refusal must say the coordinate is spent, not that it is published: {message}"
    );
}

/// The re-publish refusal is the same whichever door the publish came through.
/// The protocol handler is the one a real client uses, and it must not have its
/// own idea of what a taken coordinate means.
#[actix_web::test]
async fn the_npm_publish_endpoint_refuses_a_spent_coordinate() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(&local_svc, "local-npm", "burned", "2.0.0", json!({})).await;
    let resp = call_service(&app, bulk_delete_request("local-npm", "burned", "2.0.0")).await;
    assert_eq!(resp.status(), 200);

    let tarball = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        b"fake-tarball-content",
    );
    let req = TestRequest::put()
        .uri("/proxy/local-npm/burned")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(json!({
            "name": "burned",
            "versions": { "2.0.0": { "name": "burned", "version": "2.0.0" } },
            "_attachments": {
                "burned-2.0.0.tgz": {
                    "content_type": "application/octet-stream",
                    "data": tarball,
                    "length": 20
                }
            }
        }))
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status(),
        409,
        "npm publish onto a tombstoned coordinate must conflict"
    );
}

/// Deleting a version drops its bytes. Asserted against storage rather than
/// against a download route, so a 404 that came from the missing index row
/// cannot pass for a 404 that came from the missing blob.
#[actix_web::test]
async fn deleting_a_version_drops_its_artifact_bytes() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let storage = local_svc.storage.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    let key = publish(&local_svc, "local-npm", "byte-pkg", "1.0.0", json!({})).await;
    assert!(storage.exists(&key).await.unwrap(), "published bytes exist");

    let resp = call_service(&app, bulk_delete_request("local-npm", "byte-pkg", "1.0.0")).await;
    assert_eq!(resp.status(), 200);
    assert!(
        !storage.exists(&key).await.unwrap(),
        "the artifact must be gone: a tombstone keeps the name, not the bytes"
    );
}

/// A second delete of the same coordinate succeeds and changes nothing. Re-running
/// a partly applied bulk delete is an ordinary thing to do, and the original
/// `deleted_at` is what compaction ages against — a re-stamp would postpone it.
#[actix_web::test]
async fn deleting_twice_is_idempotent_and_keeps_the_first_timestamp() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(&local_svc, "local-npm", "twice", "1.0.0", json!({})).await;
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "twice", "1.0.0"))
            .await
            .status(),
        200
    );
    let first = local_svc
        .backend
        .find_tombstone("local-npm", "twice", "1.0.0")
        .await
        .unwrap()
        .expect("tombstone")
        .deleted_at;

    let resp = call_service(&app, bulk_delete_request("local-npm", "twice", "1.0.0")).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(
        body["succeeded"], 1,
        "deleting an already-deleted coordinate is not a failure: {body}"
    );

    let second = local_svc
        .backend
        .find_tombstone("local-npm", "twice", "1.0.0")
        .await
        .unwrap()
        .expect("tombstone")
        .deleted_at;
    assert_eq!(first, second, "the original deletion timestamp must stand");
}

/// A package *name* is a weaker claim than a version coordinate. Once every
/// version is gone the name may be published to again — but not with a number
/// that has been used.
#[actix_web::test]
async fn a_fully_deleted_name_may_be_republished_but_not_its_old_numbers() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(&local_svc, "local-npm", "recycled", "1.0.0", json!({})).await;
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "recycled", "1.0.0"))
            .await
            .status(),
        200
    );

    // The name is free.
    publish(&local_svc, "local-npm", "recycled", "2.0.0", json!({})).await;

    // The number is not.
    let err = local_svc
        .publish(PublishRequest {
            registry: "local-npm".to_owned(),
            name: "recycled".to_owned(),
            version: "1.0.0".to_owned(),
            artifact: bytes::Bytes::from_static(b"x"),
            checksum: "x".to_owned(),
            index_metadata: json!({}),
            unlisted: false,
            publisher: publisher(),
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect_err("the old version number stays spent");
    assert!(err.to_string().contains("never reused"), "{err}");
}

/// The rollback primitive must not be able to erase a tombstone. It is the only
/// path left in the tree that removes a `local_packages` row at all, and a
/// caller reaching for the wrong cleanup would silently free a spent name.
#[actix_web::test]
async fn remove_version_will_not_erase_a_tombstone() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();

    publish(&local_svc, "local-npm", "rollback", "1.0.0", json!({})).await;
    local_svc
        .backend
        .tombstone_version("local-npm", "rollback", "1.0.0", Some("admin"))
        .await
        .unwrap();

    local_svc
        .backend
        .remove_version("local-npm", "rollback", "1.0.0")
        .await
        .unwrap();

    assert!(
        local_svc
            .backend
            .find_tombstone("local-npm", "rollback", "1.0.0")
            .await
            .unwrap()
            .is_some(),
        "remove_version must leave a tombstone standing"
    );
}

// ── The tombstone is still readable ───────────────────────────────────────────

/// Absent from every listing, present to the audit view. That asymmetry is the
/// whole point: the version is not installable and must not appear to be, while
/// the question "what happened to 1.4.0" has to remain answerable.
#[actix_web::test]
async fn a_tombstone_is_listed_for_the_admin_and_nowhere_else() {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(&local_svc, "local-npm", "audited", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "audited", "2.0.0", json!({})).await;
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "audited", "1.0.0"))
            .await
            .status(),
        200
    );

    let req = TestRequest::get()
        .uri("/api/v1/admin/registries/local-npm/tombstones")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["total"], 1, "{body}");
    assert_eq!(body["tombstones"][0]["version"], "1.0.0");
    assert_eq!(body["tombstones"][0]["deleted_by"], "admin");
    assert!(
        body["tombstones"][0]["detail_compacted_at"].is_null(),
        "a fresh tombstone has not been compacted: {body}"
    );

    // …and the packument a client resolves against does not name it.
    let req = TestRequest::get()
        .uri("/proxy/local-npm/audited")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let packument: Value = read_body_json(call_service(&app, req).await).await;
    assert!(
        packument["versions"]["1.0.0"].is_null(),
        "a deleted version must not appear in the packument: {packument}"
    );
    assert!(
        !packument["versions"]["2.0.0"].is_null(),
        "the surviving version must still be there: {packument}"
    );
}

#[actix_web::test]
async fn the_tombstone_list_requires_admin() {
    let app = build_local_registry_app(
        local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None),
        Default::default(),
        None,
    )
    .await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/registries/local-npm/tombstones")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── Every ecosystem's listing ─────────────────────────────────────────────────

/// One registry type, its listing endpoint, and the metadata its reader needs.
struct EcoCase {
    /// Registry name and `type`, which the table keeps identical for clarity.
    registry: &'static str,
    kind: &'static str,
    package: &'static str,
    /// Published first and deleted. Must not appear in the listing.
    deleted: &'static str,
    /// Published second and kept. Must appear, so a listing that returns nothing
    /// at all cannot pass this test by accident.
    kept: &'static str,
    /// The protocol document a client resolves against.
    listing: &'static str,
    /// `index_metadata` for the deleted version — the shapes some readers
    /// interpolate. `%V` is replaced with the version.
    metadata: fn(&str) -> Value,
}

fn bare(_v: &str) -> Value {
    json!({})
}

/// **The `deleted_at IS NULL` sweep, asserted per registry type.**
///
/// The predicate lives in three backend methods, and every ecosystem's listing
/// is built on top of them — so in principle one fix covers all of them, and in
/// practice that is the assumption this table exists to distrust. A reader that
/// grows its own query, or an ecosystem whose listing is assembled from
/// `list_package_names` rather than `get_versions`, breaks exactly one row here.
#[actix_web::test]
async fn a_tombstoned_version_is_absent_from_every_ecosystem_listing() {
    let cases = [
        EcoCase {
            registry: "local-npm",
            kind: "npm",
            package: "listpkg",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-npm/listpkg",
            metadata: bare,
        },
        EcoCase {
            registry: "local-cargo",
            kind: "cargo",
            package: "listcrate",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-cargo/registry/li/st/listcrate",
            metadata: |v| json!({ "name": "listcrate", "vers": v, "cksum": "0", "deps": [] }),
        },
        EcoCase {
            registry: "local-nuget",
            kind: "nuget",
            package: "listlib",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-nuget/nuget/v3/flat/listlib/index.json",
            metadata: bare,
        },
        EcoCase {
            registry: "local-pypi",
            kind: "pypi",
            package: "listdist",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-pypi/simple/listdist/",
            metadata: |v| json!({ "filename": format!("listdist-{v}.tar.gz") }),
        },
        EcoCase {
            registry: "local-gems",
            kind: "rubygems",
            package: "listgem",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-gems/info/listgem",
            metadata: bare,
        },
        EcoCase {
            registry: "local-maven",
            kind: "maven",
            package: "com.example:listlib",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-maven/maven2/com/example/listlib/maven-metadata.xml",
            metadata: bare,
        },
        EcoCase {
            registry: "local-go",
            kind: "goproxy",
            package: "example.com/listmod",
            deleted: "v1.0.0",
            kept: "v2.0.0",
            listing: "/proxy/local-go/example.com/listmod/@v/list",
            // The Go version list is built from `index_metadata.Version`, not
            // from the row's own version column, so a bare `{}` would render an
            // empty list and pass the absence assertion for the wrong reason.
            metadata: |v| json!({ "Version": v }),
        },
        EcoCase {
            registry: "local-composer",
            kind: "composer",
            package: "acme/listpkg",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-composer/p2/acme/listpkg.json",
            metadata: bare,
        },
    ];

    for case in cases {
        let parts = local_registry_app_parts(case.registry, case.kind, RegistryMode::Local, None);
        let local_svc = parts.local_svc.clone();
        let app = build_local_registry_app(parts, Default::default(), None).await;

        for version in [case.deleted, case.kept] {
            publish(
                &local_svc,
                case.registry,
                case.package,
                version,
                (case.metadata)(version),
            )
            .await;
        }

        let resp = call_service(
            &app,
            bulk_delete_request(case.registry, case.package, case.deleted),
        )
        .await;
        assert_eq!(
            resp.status(),
            200,
            "{}: bulk-delete should have succeeded",
            case.kind
        );

        let req = TestRequest::get()
            .uri(case.listing)
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert_eq!(
            resp.status(),
            200,
            "{}: {} should still answer for the surviving version",
            case.kind,
            case.listing
        );
        let body = String::from_utf8_lossy(&read_body(resp).await).into_owned();

        assert!(
            body.contains(case.kept),
            "{}: the surviving version {} is missing from {} — the listing is empty, so the \
             absence assertion below would pass vacuously.\n{body}",
            case.kind,
            case.kept,
            case.listing,
        );
        assert!(
            !body.contains(case.deleted),
            "{}: the deleted version {} is still named in {}. Its bytes are gone, so every \
             client that resolves this document fails at download.\n{body}",
            case.kind,
            case.deleted,
            case.listing,
        );
    }
}

/// The package *catalogue* — the listing built from `list_package_names` rather
/// than from `get_versions` — is the other half of the sweep, and the one a fix
/// to the version query would miss. A package whose every version is deleted
/// must stop being named at all.
#[actix_web::test]
async fn a_fully_deleted_package_leaves_the_registry_catalogue() {
    let parts = local_registry_app_parts("local-composer", "composer", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;

    publish(
        &local_svc,
        "local-composer",
        "acme/gone",
        "1.0.0",
        json!({}),
    )
    .await;
    publish(
        &local_svc,
        "local-composer",
        "acme/stays",
        "1.0.0",
        json!({}),
    )
    .await;
    assert_eq!(
        call_service(
            &app,
            bulk_delete_request("local-composer", "acme/gone", "1.0.0")
        )
        .await
        .status(),
        200
    );

    let req = TestRequest::get()
        .uri("/proxy/local-composer/packages.json")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body = String::from_utf8_lossy(&read_body(resp).await).into_owned();
    assert!(body.contains("acme/stays"), "{body}");
    assert!(
        !body.contains("acme/gone"),
        "a package with no surviving version must leave the catalogue: {body}"
    );
}

// ── Compaction ────────────────────────────────────────────────────────────────

/// Build an app whose registry has a retention policy. `window` of zero makes
/// every existing tombstone immediately due, which is what lets these tests run
/// without waiting out a real window.
async fn compaction_app(
    dry_run: bool,
    window: Option<Duration>,
) -> (
    std::sync::Arc<batlehub_core::services::LocalRegistryService>,
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
) {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    if let Some(window) = window {
        local_svc.hot.write().await.retention.insert(
            "local-npm".to_owned(),
            RetentionPolicy {
                tombstone_detail_for: Some(window),
                dry_run,
            },
        );
    }
    let app = build_local_registry_app(parts, Default::default(), None).await;
    (local_svc, app)
}

fn compact_request(dry_run: Option<bool>) -> actix_http::Request {
    let uri = match dry_run {
        Some(v) => format!("/api/v1/admin/registries/local-npm/tombstones/compact?dry_run={v}"),
        None => "/api/v1/admin/registries/local-npm/tombstones/compact".to_owned(),
    };
    TestRequest::post()
        .uri(&uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request()
}

/// Compaction strips the detail and keeps the claim. Both halves matter: losing
/// the checksum is the point, and keeping the refusal is the invariant.
#[actix_web::test]
async fn compaction_strips_detail_and_keeps_the_claim() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;

    publish(&local_svc, "local-npm", "old", "1.0.0", json!({"a": 1})).await;
    local_svc
        .backend
        .tombstone_version("local-npm", "old", "1.0.0", Some("admin-1"))
        .await
        .unwrap();

    let resp = call_service(&app, compact_request(None)).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["compacted"], 1, "{body}");
    assert_eq!(body["dry_run"], false, "{body}");
    assert_eq!(body["coordinates"][0], "old@1.0.0", "{body}");

    let ts = local_svc
        .backend
        .find_tombstone("local-npm", "old", "1.0.0")
        .await
        .unwrap()
        .expect("the row survives compaction — that is the whole design");
    assert!(ts.detail_compacted_at.is_some(), "compaction is recorded");
    assert!(ts.checksum.is_none(), "the checksum is detail and is gone");
    assert!(ts.published_by.is_none(), "the publisher is detail too");
    assert_eq!(
        ts.deleted_by.as_deref(),
        Some("admin-1"),
        "who deleted it is part of the claim, not the detail"
    );

    // The claim still refuses a re-publish, which is the reason the row is kept.
    let err = local_svc
        .publish(PublishRequest {
            registry: "local-npm".to_owned(),
            name: "old".to_owned(),
            version: "1.0.0".to_owned(),
            artifact: bytes::Bytes::from_static(b"x"),
            checksum: "x".to_owned(),
            index_metadata: json!({}),
            unlisted: false,
            publisher: publisher(),
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect_err("a compacted tombstone still spends its coordinate");
    assert!(err.to_string().contains("never reused"), "{err}");
}

/// A dry run reports and writes nothing, and reports the same coordinates the
/// live run then strips.
#[actix_web::test]
async fn compaction_dry_run_writes_nothing_and_agrees_with_the_live_run() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;

    publish(&local_svc, "local-npm", "dry", "1.0.0", json!({})).await;
    local_svc
        .backend
        .tombstone_version("local-npm", "dry", "1.0.0", Some("admin-1"))
        .await
        .unwrap();

    let preview: Value =
        read_body_json(call_service(&app, compact_request(Some(true))).await).await;
    assert_eq!(preview["dry_run"], true, "{preview}");
    assert_eq!(preview["compacted"], 1, "{preview}");
    assert!(
        local_svc
            .backend
            .find_tombstone("local-npm", "dry", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .checksum
            .is_some(),
        "a dry run must not have stripped anything"
    );

    let live: Value = read_body_json(call_service(&app, compact_request(None)).await).await;
    assert_eq!(live["dry_run"], false, "{live}");
    assert_eq!(
        live["coordinates"], preview["coordinates"],
        "the live run must strip exactly what the dry run promised"
    );
}

/// A configured `dry_run = true` is an operator's safety catch, and a query
/// string must not take it off.
#[actix_web::test]
async fn a_configured_dry_run_cannot_be_overridden_by_the_request() {
    let (local_svc, app) = compaction_app(true, Some(Duration::from_secs(0))).await;

    publish(&local_svc, "local-npm", "safe", "1.0.0", json!({})).await;
    local_svc
        .backend
        .tombstone_version("local-npm", "safe", "1.0.0", Some("admin-1"))
        .await
        .unwrap();

    let body: Value = read_body_json(call_service(&app, compact_request(Some(false))).await).await;
    assert_eq!(
        body["dry_run"], true,
        "dry_run=false in the query must not disarm a configured dry run: {body}"
    );
    assert!(
        local_svc
            .backend
            .find_tombstone("local-npm", "safe", "1.0.0")
            .await
            .unwrap()
            .unwrap()
            .checksum
            .is_some(),
        "nothing may have been stripped"
    );
}

/// Compaction never touches a live row. Asserted by running it against a
/// registry whose versions are all live and comparing what comes back.
#[actix_web::test]
async fn compaction_never_touches_a_live_row() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;

    publish(&local_svc, "local-npm", "alive", "1.0.0", json!({"k": "v"})).await;
    let before = local_svc
        .backend
        .get_versions("local-npm", "alive")
        .await
        .unwrap();

    let body: Value = read_body_json(call_service(&app, compact_request(None)).await).await;
    assert_eq!(body["compacted"], 0, "{body}");

    let after = local_svc
        .backend
        .get_versions("local-npm", "alive")
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].checksum, before[0].checksum);
    assert_eq!(after[0].index_metadata, before[0].index_metadata);
    assert_eq!(after[0].published_by, before[0].published_by);
}

/// A tombstone inside the window keeps its detail, so the window is doing
/// something rather than the run stripping everything it can reach.
#[actix_web::test]
async fn compaction_respects_the_window() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(3600))).await;

    publish(&local_svc, "local-npm", "recent", "1.0.0", json!({})).await;
    local_svc
        .backend
        .tombstone_version("local-npm", "recent", "1.0.0", Some("admin-1"))
        .await
        .unwrap();

    let body: Value = read_body_json(call_service(&app, compact_request(None)).await).await;
    assert_eq!(
        body["compacted"], 0,
        "a tombstone deleted seconds ago is not an hour old: {body}"
    );
    assert_eq!(body["skipped"], 1, "{body}");
}

/// With no window configured the endpoint refuses rather than reporting a
/// successful run that stripped nothing — an operator calling it believes they
/// are reclaiming space, and `200 {"compacted": 0}` would confirm it.
#[actix_web::test]
async fn compaction_is_a_conflict_when_nothing_is_configured() {
    let (_local_svc, app) = compaction_app(false, None).await;
    let resp = call_service(&app, compact_request(None)).await;
    assert_eq!(resp.status(), 409);
}

#[actix_web::test]
async fn compaction_requires_admin() {
    let (_local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/tombstones/compact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
