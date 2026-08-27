//! Per-package **visibility** on the artifact routes that used to read storage
//! directly.
//!
//! The 2026-08-26 survey's findings 6 and 7 are the half of the local-read gap
//! that fails the other way round from findings 4, 5, 9 and 10: these three
//! routes ran the registry rule chain but never called `check_visibility`,
//! because they built a storage key and read `local_svc.storage` themselves
//! rather than going through `get_artifact`. The listing beside each one *did*
//! check, so the symptom was an asymmetry — `maven-metadata.xml` refused a
//! team-visibility coordinate while the jar next to it was served to anyone.
//!
//! [`crates/web/tests/authz_matrix.rs`] covers the same three routes on its
//! visibility axis, but deliberately with `Visibility::Internal`, which refuses
//! anything below `User` and so is answered by the role alone. `Team` is the
//! case that needs a *group*: an authenticated `User` who is not a member is
//! exactly the caller an Internal check waves through, and is the one an
//! operator sets team visibility to stop. That is what these tests hold.
//!
//! Each has its public-visibility control immediately before the assertion, so
//! a test that starts passing because the fixture stopped publishing is visible
//! as a failure rather than as a pass.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, TestRequest};

use batlehub_adapters::in_memory::InMemoryTeamNamespaceStore;
use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{TeamNamespace, Visibility};
use batlehub_core::ports::TeamNamespacePort;

/// A local-mode registry of `kind` with a team-namespace port wired in.
///
/// The port is the fixture: `check_visibility` returns `Ok(())` outright when
/// there is none, so without it every assertion here would pass vacuously.
async fn app_with_namespaces(
    name: &'static str,
    kind: &'static str,
) -> (
    impl TestService,
    Arc<InMemoryTeamNamespaceStore>,
    Arc<batlehub_core::services::LocalRegistryService>,
) {
    let ns_store = InMemoryTeamNamespaceStore::new();
    let mut parts = local_only_app_parts(name, kind, RegistryMode::Local, false);

    let cur = parts.local_svc.clone();
    parts.local_svc = Arc::new(batlehub_core::services::LocalRegistryService {
        backend: cur.backend.clone(),
        storage: cur.storage.clone(),
        hot: cur.hot.clone(),
        quota: cur.quota.clone(),
        ownership: cur.ownership.clone(),
        team_namespace: Some(ns_store.clone() as Arc<dyn TeamNamespacePort>),
        sbom: cur.sbom.clone(),
        explore_cache: cur.explore_cache.clone(),
        package_repo: cur.package_repo.clone(),
        readme: cur.readme.clone(),
    });

    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;
    (app, ns_store, local_svc)
}

/// Claim `prefix` for a group `USER_TOKEN` is not in, and mark `package` as
/// team-visible.
///
/// Both halves are needed: `check_team_visibility` refuses when it cannot find
/// an owning namespace *and* when the caller is not in the one it finds, and a
/// test that only set the visibility would be exercising the first.
async fn make_team_only(ns_store: &InMemoryTeamNamespaceStore, registry: &str, package: &str) {
    ns_store
        .claim_namespace(TeamNamespace {
            registry: registry.to_owned(),
            prefix: package.to_owned(),
            group_id: "team-nobody".to_owned(),
            claimed_by: Some("admin".to_owned()),
        })
        .await
        .expect("claim namespace");
    ns_store
        .set_visibility(registry, package, Visibility::Team)
        .await
        .expect("set visibility");
}

async fn get_as<S: TestService>(app: &S, uri: &str, token: &str) -> u16 {
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(token)))
        .to_request();
    call_service(app, req).await.status().as_u16()
}

// ── Maven (survey finding 6) ─────────────────────────────────────────────────

const SAMPLE_POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>mylib</artifactId>
  <version>1.0.0</version>
  <packaging>jar</packaging>
</project>"#;

/// The jar is stored under `maven_artifact_storage_key`, not the flat artifact
/// key, which is why this route computed its own key and why it ended up
/// bypassing the visibility check that `maven-metadata.xml` runs.
#[actix_web::test]
async fn maven_jar_is_refused_to_a_non_member_of_a_team_visible_coordinate() {
    let (app, ns_store, _local_svc) = app_with_namespaces("local-maven", "maven").await;

    for (uri, payload) in [
        (
            "/proxy/local-maven/maven2/com/example/mylib/1.0.0/mylib-1.0.0.pom",
            SAMPLE_POM.as_bytes(),
        ),
        (
            "/proxy/local-maven/maven2/com/example/mylib/1.0.0/mylib-1.0.0.jar",
            b"fake-jar-bytes".as_slice(),
        ),
    ] {
        let req = TestRequest::put()
            .uri(uri)
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .set_payload(payload)
            .to_request();
        assert_eq!(call_service(&app, req).await.status(), 201, "{uri}");
    }

    let jar = "/proxy/local-maven/maven2/com/example/mylib/1.0.0/mylib-1.0.0.jar";
    let metadata = "/proxy/local-maven/maven2/com/example/mylib/maven-metadata.xml";

    // Control: public, so both are served.
    assert_eq!(get_as(&app, jar, USER_TOKEN).await, 200);
    assert_eq!(get_as(&app, metadata, USER_TOKEN).await, 200);

    make_team_only(&ns_store, "local-maven", "com.example:mylib").await;

    // The listing always refused. The jar is the half that did not.
    assert_eq!(get_as(&app, metadata, USER_TOKEN).await, 403);
    assert_eq!(
        get_as(&app, jar, USER_TOKEN).await,
        403,
        "the jar must not outlive the metadata's refusal"
    );
}

// ── NuGet (survey finding 6) ─────────────────────────────────────────────────

fn make_sample_nupkg(id: &str, version: &str) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let nuspec = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd">
  <metadata>
    <id>{id}</id>
    <version>{version}</version>
    <description>a package</description>
    <authors>TestAuthor</authors>
  </metadata>
</package>"#
    );
    let mut buf = Vec::new();
    let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
    let opts = SimpleFileOptions::default();
    zip.start_file(format!("{id}.nuspec"), opts).unwrap();
    zip.write_all(nuspec.as_bytes()).unwrap();
    zip.finish().unwrap();
    buf
}

#[actix_web::test]
async fn nupkg_is_refused_to_a_non_member_of_a_team_visible_package() {
    let (app, ns_store, _local_svc) = app_with_namespaces("local-nuget", "nuget").await;

    let boundary = "nugetboundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; filename=\"package.nupkg\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    ).into_bytes();
    body.extend_from_slice(&make_sample_nupkg("mylib", "1.0.0"));
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let req = TestRequest::put()
        .uri("/proxy/local-nuget/nuget/api/v2/package")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header((
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        ))
        .set_payload(body)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let nupkg = "/proxy/local-nuget/nuget/v3/flat/mylib/1.0.0/mylib.1.0.0.nupkg";
    let index = "/proxy/local-nuget/nuget/v3/flat/mylib/index.json";

    assert_eq!(get_as(&app, nupkg, USER_TOKEN).await, 200);
    assert_eq!(get_as(&app, index, USER_TOKEN).await, 200);

    make_team_only(&ns_store, "local-nuget", "mylib").await;

    assert_eq!(get_as(&app, index, USER_TOKEN).await, 403);
    assert_eq!(
        get_as(&app, nupkg, USER_TOKEN).await,
        403,
        "the flat index refused this caller; the bytes it points at must too"
    );
}

// ── Terraform provider binary (survey finding 7) ─────────────────────────────

const PROVIDER_MANIFEST: &str = r#"{
  "version": "5.0.0",
  "protocols": ["5.0"],
  "platforms": [
    {"os": "linux", "arch": "amd64", "filename": "terraform-provider-aws_5.0.0_linux_amd64.zip", "shasum": "deadbeef"}
  ]
}"#;

/// Terraform is local-only, so this route has no proxy fall-through to fall back
/// on: the gate it applies is the only gate the provider zip ever gets.
#[actix_web::test]
async fn provider_binary_is_refused_to_a_non_member_of_a_team_visible_provider() {
    let (app, ns_store, _local_svc) = app_with_namespaces("local-tf", "terraform").await;

    let req = TestRequest::post()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/versions")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(PROVIDER_MANIFEST)
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 201);

    let req = TestRequest::put()
        .uri("/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .set_payload(b"fake-zip-bytes".as_slice())
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    let binary = "/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/artifact/linux/amd64";
    let versions = "/proxy/local-tf/v1/providers/hashicorp/aws/versions";
    let download = "/proxy/local-tf/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64";

    assert_eq!(get_as(&app, binary, USER_TOKEN).await, 200);
    assert_eq!(get_as(&app, versions, USER_TOKEN).await, 200);

    make_team_only(&ns_store, "local-tf", "providers/hashicorp/aws").await;

    // The two documents that *describe* the download always refused. The
    // download itself is the one a caller could reach by constructing its URL.
    assert_eq!(get_as(&app, versions, USER_TOKEN).await, 403);
    assert_eq!(get_as(&app, download, USER_TOKEN).await, 403);
    assert_eq!(
        get_as(&app, binary, USER_TOKEN).await,
        403,
        "refused the listing and the download document, but handed over the zip"
    );
}

// ── Search (survey finding 11) ───────────────────────────────────────────────

/// Publish `name` straight through the service, as `authz_matrix` seeds.
async fn seed(
    local_svc: &batlehub_core::services::LocalRegistryService,
    registry: &str,
    name: &str,
) {
    use batlehub_core::entities::{Identity, Role};
    use bytes::Bytes;
    use sha2::{Digest, Sha256};

    let artifact = Bytes::from_static(b"search-fixture-bytes");
    let checksum = hex::encode(Sha256::digest(&artifact));
    local_svc
        .publish(batlehub_core::services::PublishRequest {
            registry: registry.to_owned(),
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            artifact,
            checksum,
            index_metadata: serde_json::json!({}),
            unlisted: false,
            publisher: Identity {
                user_id: Some("user-1".to_owned()),
                role: Role::User,
                auth_provider: None,
                groups: vec![],
            },
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect("fixture must publish");
}

async fn npm_search_names<S: TestService>(app: &S, token: &str) -> Vec<String> {
    let req = TestRequest::get()
        .uri("/proxy/local-npm/-/v1/search?text=acme&size=50")
        .insert_header(("Authorization", bearer(token)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&actix_web::test::read_body(resp).await).expect("JSON");
    body["objects"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| o["package"]["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Survey finding 11. `local_hits` read `list_package_names` — a bare
/// `SELECT DISTINCT name FROM local_packages` with no visibility, `unlisted` or
/// identity filter — so a search named every private package in the registry to
/// anyone who asked, including callers the package itself answers `403` to.
///
/// The control is the same request as an admin: if the search returned nothing
/// to anybody the assertion below would pass for the wrong reason.
#[actix_web::test]
async fn search_does_not_name_a_team_visible_package_to_a_non_member() {
    let (app, ns_store, local_svc) = app_with_namespaces("local-npm", "npm").await;
    seed(&local_svc, "local-npm", "acme-public").await;
    seed(&local_svc, "local-npm", "acme-secret").await;

    // Control: both are public, so both are found.
    let before = npm_search_names(&app, USER_TOKEN).await;
    assert!(before.contains(&"acme-secret".to_owned()), "{before:?}");

    make_team_only(&ns_store, "local-npm", "acme-secret").await;

    let names = npm_search_names(&app, USER_TOKEN).await;
    assert!(
        names.contains(&"acme-public".to_owned()),
        "the public package must still be found: {names:?}"
    );
    assert!(
        !names.contains(&"acme-secret".to_owned()),
        "search named a team-visible package to a non-member: {names:?}"
    );

    // …and an admin, who is above visibility, still sees it. Without this the
    // test would pass against a search that returns nothing at all.
    let as_admin = npm_search_names(&app, ADMIN_TOKEN).await;
    assert!(as_admin.contains(&"acme-secret".to_owned()), "{as_admin:?}");
}

/// The other half of finding 11, and the one filtering alone does not close.
///
/// The search cache is keyed `search:{registry}:{limit}:{query}` — no identity
/// in it — and the merged result set used to be what got stored. So the *first*
/// authorised searcher wrote their private hits into a shared entry, and every
/// later caller of the same query was served them from rung 1, filter or no
/// filter. The cache now holds the upstream answer only; the local half is
/// merged per request.
#[actix_web::test]
async fn a_warmed_search_cache_does_not_replay_one_callers_private_hits_to_another() {
    let ns_store = InMemoryTeamNamespaceStore::new();
    let mut parts = local_only_app_parts("hybrid-npm", "npm", RegistryMode::Hybrid, true);
    let cur = parts.local_svc.clone();
    parts.local_svc = Arc::new(batlehub_core::services::LocalRegistryService {
        backend: cur.backend.clone(),
        storage: cur.storage.clone(),
        hot: cur.hot.clone(),
        quota: cur.quota.clone(),
        ownership: cur.ownership.clone(),
        team_namespace: Some(ns_store.clone() as Arc<dyn TeamNamespacePort>),
        sbom: cur.sbom.clone(),
        explore_cache: cur.explore_cache.clone(),
        package_repo: cur.package_repo.clone(),
        readme: cur.readme.clone(),
    });
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    seed(&local_svc, "hybrid-npm", "acme-secret").await;
    make_team_only(&ns_store, "hybrid-npm", "acme-secret").await;

    let names_for = |token: &'static str| {
        let app = &app;
        async move {
            let req = TestRequest::get()
                .uri("/proxy/hybrid-npm/-/v1/search?text=acme&size=50")
                .insert_header(("Authorization", bearer(token)))
                .to_request();
            let resp = call_service(app, req).await;
            assert_eq!(resp.status(), 200);
            let body: serde_json::Value =
                serde_json::from_slice(&actix_web::test::read_body(resp).await).expect("JSON");
            body["objects"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o["package"]["name"].as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
    };

    // The admin may see it, and warms the cache doing so.
    let as_admin = names_for(ADMIN_TOKEN).await;
    assert!(as_admin.contains(&"acme-secret".to_owned()), "{as_admin:?}");

    // The same query, from a caller who may not.
    let as_user = names_for(USER_TOKEN).await;
    assert!(
        !as_user.contains(&"acme-secret".to_owned()),
        "the cache replayed the admin's private hit to a non-member: {as_user:?}"
    );
}
