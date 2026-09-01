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
use batlehub_core::entities::{AccessAction, EventFilter, Identity, Role};
use batlehub_core::ports::OwnershipPort as _;
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

/// A local-npm app with an ownership store wired in, handed back so a test can
/// inspect it.
///
/// A separate builder because the shared factory leaves `ownership: None`, and
/// that is not a neutral default for these two tests: with the port absent there
/// are no grants to outlive anything, and both would pass without exercising a
/// single check.
async fn ownership_app() -> (
    std::sync::Arc<batlehub_core::services::LocalRegistryService>,
    std::sync::Arc<batlehub_adapters::in_memory::InMemoryOwnershipStore>,
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
) {
    let mut parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let ownership = batlehub_adapters::in_memory::InMemoryOwnershipStore::new();
    let cur = parts.local_svc.clone();
    parts.local_svc = std::sync::Arc::new(batlehub_core::services::LocalRegistryService {
        backend: cur.backend.clone(),
        storage: cur.storage.clone(),
        hot: cur.hot.clone(),
        quota: cur.quota.clone(),
        ownership: Some(
            ownership.clone() as std::sync::Arc<dyn batlehub_core::ports::OwnershipPort>
        ),
        team_namespace: cur.team_namespace.clone(),
        sbom: cur.sbom.clone(),
        explore_cache: cur.explore_cache.clone(),
        package_repo: cur.package_repo.clone(),
        readme: cur.readme.clone(),
    });
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, Default::default(), None).await;
    (local_svc, ownership, app)
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

/// **Package-tier grants do not outlive their package** (RFC 0016 §4.4, §7).
///
/// The name is released when its last version goes, so someone else may take it.
/// If the owner rows survived that, the previous owner would still hold publish
/// and owner-management authority over a package they have never seen — a claim
/// on a name nobody currently owns, arriving through the back door. The version
/// tombstones stay, because they are the invariant; the grants go, because they
/// are a decision about a thing that no longer exists.
#[actix_web::test]
async fn deleting_the_last_version_takes_the_package_grants_with_it() {
    let (local_svc, ownership, app) = ownership_app().await;

    publish(&local_svc, "local-npm", "handover", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "handover", "2.0.0", json!({})).await;
    assert!(
        !ownership
            .list_owners("local-npm", "handover")
            .await
            .unwrap()
            .is_empty(),
        "the first publish registers its publisher as owner"
    );

    // One of two versions: the package still exists, so its grants stand.
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "handover", "1.0.0"))
            .await
            .status(),
        200
    );
    assert!(
        !ownership
            .list_owners("local-npm", "handover")
            .await
            .unwrap()
            .is_empty(),
        "a package with a surviving version keeps its owners"
    );

    // The last one: the name is released, and the grants go with it.
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "handover", "2.0.0"))
            .await
            .status(),
        200
    );
    assert!(
        ownership
            .list_owners("local-npm", "handover")
            .await
            .unwrap()
            .is_empty(),
        "the last version's deletion releases the name, so nothing may still claim it"
    );
}

/// The consequence of the above, from the side that matters: a *different*
/// principal takes the released name and the previous owner has no say in it.
#[actix_web::test]
async fn a_previous_owner_holds_nothing_over_a_released_name() {
    let (local_svc, ownership, app) = ownership_app().await;

    publish(&local_svc, "local-npm", "released", "1.0.0", json!({})).await;
    assert_eq!(
        call_service(&app, bulk_delete_request("local-npm", "released", "1.0.0"))
            .await
            .status(),
        200
    );

    // Someone else creates the name.
    let newcomer = Identity {
        user_id: Some("user-2".to_owned()),
        role: Role::User,
        auth_provider: None,
        groups: vec![],
    };
    local_svc
        .publish(PublishRequest {
            registry: "local-npm".to_owned(),
            name: "released".to_owned(),
            version: "9.0.0".to_owned(),
            artifact: bytes::Bytes::from_static(b"new-owner-bytes"),
            checksum: "c".to_owned(),
            index_metadata: json!({}),
            unlisted: false,
            publisher: newcomer.clone(),
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect("the released name may be taken");

    let owners = ownership
        .list_owners("local-npm", "released")
        .await
        .unwrap();
    assert_eq!(
        owners.len(),
        1,
        "exactly one owner — the newcomer. A surviving row for user-1 is authority \
         over a package they have never seen: {owners:?}"
    );
    assert_eq!(owners[0].principal_id, "user-2");
    assert!(
        !ownership
            .can_publish("local-npm", "released", &publisher())
            .await
            .unwrap(),
        "the previous owner must not be able to publish to the new owner's package"
    );
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
        // The package name is the storage-side coordinate the handler builds
        // (`modules/{namespace}/{name}/{provider}`), not the request path.
        EcoCase {
            registry: "local-tf",
            kind: "terraform",
            package: "modules/hashicorp/listmod/aws",
            deleted: "0.1.0",
            kept: "0.2.0",
            listing: "/proxy/local-tf/v1/modules/hashicorp/listmod/aws/versions",
            metadata: bare,
        },
        EcoCase {
            registry: "local-tf-prov",
            kind: "terraform",
            package: "providers/hashicorp/listprov",
            deleted: "1.0.0",
            kept: "2.0.0",
            listing: "/proxy/local-tf-prov/v1/providers/hashicorp/listprov/versions",
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
                ..Default::default()
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

// ── Retention (RFC 0016 §4.1–4.3) ─────────────────────────────────────────────

/// A registry with a retention policy, plus the store the download signal is
/// recorded into so a test can make a version look used.
///
/// The `AdminService` comes back too: it is the same `PackageRepository` the
/// audit trail is written through, so a test can read back what a run recorded.
async fn retention_app(
    policy: RetentionPolicy,
) -> (
    std::sync::Arc<batlehub_core::services::LocalRegistryService>,
    std::sync::Arc<batlehub_core::services::AdminService>,
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
) {
    let parts = local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let admin_svc = parts.admin_svc.clone();
    local_svc
        .hot
        .write()
        .await
        .retention
        .insert("local-npm".to_owned(), policy);
    let app = build_local_registry_app(parts, Default::default(), None).await;
    (local_svc, admin_svc, app)
}

/// Every audited action a run left behind, newest first.
async fn recorded_actions(
    admin_svc: &batlehub_core::services::AdminService,
    action: AccessAction,
) -> Vec<batlehub_core::entities::AccessEvent> {
    admin_svc
        .list_events(EventFilter {
            actions: vec![action],
            limit: 100,
            ..Default::default()
        })
        .await
        .unwrap()
}

fn retention_request(dry_run: Option<bool>) -> actix_http::Request {
    let uri = match dry_run {
        Some(v) => format!("/api/v1/admin/registries/local-npm/retention?dry_run={v}"),
        None => "/api/v1/admin/registries/local-npm/retention".to_owned(),
    };
    TestRequest::post()
        .uri(&uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request()
}

/// A live run reclaims through the same path a hand deletion takes: the bytes
/// go, the listing loses the version, and the coordinate is spent.
#[actix_web::test]
async fn a_reclaimed_version_is_deleted_the_same_way_a_hand_deletion_is() {
    let (local_svc, admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        dry_run: false,
        ..Default::default()
    })
    .await;
    let storage = local_svc.storage.clone();

    let old = publish(&local_svc, "local-npm", "p", "1.0.0", json!({})).await;
    let new = publish(&local_svc, "local-npm", "p", "2.0.0", json!({})).await;

    let resp = call_service(&app, retention_request(None)).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["reclaimed"], 1, "{body}");
    assert_eq!(body["dry_run"], false, "{body}");
    assert_eq!(body["reclaimed_coordinates"][0], "p@1.0.0", "{body}");

    assert!(!storage.exists(&old).await.unwrap(), "the bytes are gone");
    assert!(
        storage.exists(&new).await.unwrap(),
        "the kept one is intact"
    );
    assert!(
        local_svc
            .backend
            .find_tombstone("local-npm", "p", "1.0.0")
            .await
            .unwrap()
            .is_some(),
        "a reclamation spends the coordinate, exactly as a deletion does"
    );

    // …and it is the one thing that must *not* look the same: the trail has to
    // separate a policy from a person (RFC 0016 §3).
    let reclaims = recorded_actions(&admin_svc, AccessAction::RetentionReclaim).await;
    assert_eq!(reclaims.len(), 1);
    let coord = reclaims[0].package_id.as_ref().unwrap();
    assert_eq!(coord.name, "p");
    assert_eq!(coord.version, "1.0.0");
    assert!(
        recorded_actions(&admin_svc, AccessAction::Delete)
            .await
            .is_empty(),
        "a hand deletion is a different event"
    );
    assert_eq!(
        recorded_actions(&admin_svc, AccessAction::RetentionRun)
            .await
            .len(),
        1,
        "and the run itself is on the record"
    );
}

/// A dry run is a decision an operator made against a production registry, and
/// the only record it used to leave was a response body nobody keeps.
///
/// It records the run and nothing else: no `retention_reclaim` for a version
/// that is still there, or the action stops meaning "this version is gone".
#[actix_web::test]
async fn a_retention_dry_run_is_on_the_record_without_faking_a_deletion() {
    let (local_svc, admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        dry_run: true,
        ..Default::default()
    })
    .await;
    publish(&local_svc, "local-npm", "p", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "p", "2.0.0", json!({})).await;

    let body: Value = read_body_json(call_service(&app, retention_request(None)).await).await;
    assert_eq!(body["dry_run"], true, "{body}");
    assert_eq!(body["reclaimed_coordinates"][0], "p@1.0.0", "{body}");

    assert_eq!(
        recorded_actions(&admin_svc, AccessAction::RetentionDryRun)
            .await
            .len(),
        1,
        "who previewed the policy, and when"
    );
    assert!(
        recorded_actions(&admin_svc, AccessAction::RetentionRun)
            .await
            .is_empty(),
        "a preview must never be filed as a run that could have written"
    );
    assert!(
        recorded_actions(&admin_svc, AccessAction::RetentionReclaim)
            .await
            .is_empty(),
        "nothing was reclaimed, so nothing may say it was"
    );
    assert!(local_svc
        .backend
        .find_tombstone("local-npm", "p", "1.0.0")
        .await
        .unwrap()
        .is_none());
}

/// A configured `dry_run = true` is an operator's safety catch, and a query
/// string must not take it off — the same interlock compaction has.
#[actix_web::test]
async fn a_configured_retention_dry_run_cannot_be_overridden_by_the_request() {
    let (local_svc, _admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        dry_run: true,
        ..Default::default()
    })
    .await;
    publish(&local_svc, "local-npm", "p", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "p", "2.0.0", json!({})).await;

    let body: Value =
        read_body_json(call_service(&app, retention_request(Some(false))).await).await;
    assert_eq!(
        body["dry_run"], true,
        "dry_run=false must not disarm a configured dry run: {body}"
    );
    assert_eq!(body["reclaimed"], 1, "it still reports what it would do");
    assert_eq!(
        local_svc
            .backend
            .get_versions("local-npm", "p")
            .await
            .unwrap()
            .len(),
        2,
        "nothing may have been reclaimed"
    );
}

/// The version-tier pin, end to end through its own endpoint.
#[actix_web::test]
async fn a_pinned_version_survives_a_live_run() {
    let (local_svc, _admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        dry_run: false,
        ..Default::default()
    })
    .await;
    publish(&local_svc, "local-npm", "p", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "p", "2.0.0", json!({})).await;

    let pin = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/retention-pin")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(json!({ "name": "p", "version": "1.0.0", "keep": true }))
        .to_request();
    assert_eq!(call_service(&app, pin).await.status(), 200);

    let body: Value = read_body_json(call_service(&app, retention_request(None)).await).await;
    assert_eq!(
        body["reclaimed"], 0,
        "the pin outranks the policy above it: {body}"
    );
    let pinned = body["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["version"] == "1.0.0")
        .unwrap();
    assert_eq!(pinned["kept_because"], "pinned", "{body}");
}

/// Unpinning gives the policy back its say.
#[actix_web::test]
async fn unpinning_lets_the_policy_reclaim_again() {
    let (local_svc, _admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        dry_run: false,
        ..Default::default()
    })
    .await;
    publish(&local_svc, "local-npm", "p", "1.0.0", json!({})).await;
    publish(&local_svc, "local-npm", "p", "2.0.0", json!({})).await;

    for keep in [true, false] {
        let req = TestRequest::post()
            .uri("/api/v1/admin/registries/local-npm/retention-pin")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .set_json(json!({ "name": "p", "version": "1.0.0", "keep": keep }))
            .to_request();
        assert_eq!(call_service(&app, req).await.status(), 200);
    }

    let body: Value = read_body_json(call_service(&app, retention_request(None)).await).await;
    assert_eq!(body["reclaimed_coordinates"][0], "p@1.0.0", "{body}");
}

/// An unconfigured registry gets a `409`, not a run that found nothing: an
/// operator calling this believes they are reclaiming space.
#[actix_web::test]
async fn retention_is_a_conflict_when_no_keep_condition_is_configured() {
    let app = build_local_registry_app(
        local_registry_app_parts("local-npm", "npm", RegistryMode::Local, None),
        Default::default(),
        None,
    )
    .await;
    assert_eq!(
        call_service(&app, retention_request(None)).await.status(),
        409
    );

    // …and the same for a block that only configures compaction.
    let (_svc, _admin_svc, app) = retention_app(RetentionPolicy {
        tombstone_detail_for: Some(Duration::from_secs(0)),
        ..Default::default()
    })
    .await;
    assert_eq!(
        call_service(&app, retention_request(None)).await.status(),
        409
    );
}

#[actix_web::test]
async fn retention_requires_admin() {
    let (_svc, _admin_svc, app) = retention_app(RetentionPolicy {
        keep_versions: Some(1),
        ..Default::default()
    })
    .await;
    for uri in [
        "/api/v1/admin/registries/local-npm/retention",
        "/api/v1/admin/registries/local-npm/retention-pin",
    ] {
        let req = TestRequest::post()
            .uri(uri)
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .set_json(json!({ "name": "p", "version": "1.0.0", "keep": true }))
            .to_request();
        assert_eq!(call_service(&app, req).await.status(), 403, "{uri}");
    }
}

// ── Are the control verbs actually delegable? ────────────────────────────────
//
// RFC 0015 §13.12 claims the `require_admin` decomposition made "each verb
// delegable" — that is the whole return on splitting one helper into thirteen
// verbs across 98 call sites. `compaction_requires_admin` above cannot check it:
// `role:user` holds `tombstones:read` under no translation rule, so that test
// passes whether the decision comes from the engine or from a role assertion
// behind it. These two rows are the difference, and they are the shape §13.8
// named — *a role assertion in front of the engine silently overrides the config
// it is supposed to enforce.*

/// Grant `action` to `role:user` on `local-npm`, on top of the fixture's own
/// hierarchy, exactly as `[registries.grants]` would.
async fn grant_to_user(
    local_svc: &batlehub_core::services::LocalRegistryService,
    action: batlehub_core::entities::Action,
) {
    use batlehub_core::entities::{GrantMap, Role, SubjectMatcher};
    use std::sync::Arc;

    let mut hot = local_svc.hot.write().await;
    let mut grants = (**hot
        .grants
        .get("local-npm")
        .expect("the fixture must have a hierarchy for this to add to"))
    .clone();

    // Added to the registry node's own map, which is what `[registries.grants]`
    // writes to — not a replacement of it, since replacement is revocation under
    // another name (§4.3).
    let map = grants
        .registry
        .grants
        .take()
        .unwrap_or_else(GrantMap::new)
        .grant(SubjectMatcher::Role(Role::User), [action]);
    grants.registry.grants = Some(map);

    hot.grants.insert("local-npm".to_owned(), Arc::new(grants));
}

/// An operator delegating `tombstones:read` must actually delegate it.
#[actix_web::test]
async fn tombstones_read_granted_to_a_user_is_honoured() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;
    grant_to_user(&local_svc, batlehub_core::entities::Action::TombstonesRead).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/tombstones/compact")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status(),
        200,
        "the grant resolves and the handler's `require_verb` passes; a `403` here \
         is a role assertion deeper in overriding the config that was written to \
         permit this, which is exactly what §6.1 deleted from the publish path"
    );
}

/// The same question for `retention:run`, whose floor is `Role::User` rather
/// than `Role::Admin` — a lower floor, and still a floor.
#[actix_web::test]
async fn retention_run_granted_to_a_user_is_honoured() {
    let (local_svc, app) = compaction_app(false, Some(Duration::from_secs(0))).await;
    grant_to_user(&local_svc, batlehub_core::entities::Action::RetentionRun).await;
    publish(&local_svc, "local-npm", "pinme", "1.0.0", json!({})).await;

    let req = TestRequest::post()
        .uri("/api/v1/admin/registries/local-npm/retention-pin")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_json(json!({ "name": "pinme", "version": "1.0.0", "keep": true }))
        .to_request();
    let status = call_service(&app, req).await.status();
    assert!(
        status.is_success(),
        "granted `retention:run` and got {status}"
    );
}
