//! Integration tests split from the former monolithic `integration.rs`
//! (see `tests/common/mod.rs` for shared app-factory infrastructure).

mod common;
#[allow(unused_imports)]
use common::*;

use std::collections::HashMap;
use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_adapters::cache::InMemoryCacheStore;
use batlehub_adapters::in_memory::{
    InMemoryPackageRepository as InMemoryRepo, InMemoryStorageBackend as InMemoryStorage,
    NoopArtifactMetaRepository as NoopArtifactMeta, NullUserTokenRepository as NullTokenRepository,
};
use batlehub_core::{
    ports::{CacheStore, PackageRepository, RegistryClient, StorageBackend, UserTokenRepository},
    services::{new_hot_lock, AdminService, HotConfig, ProxyMetrics, ProxyService, RegistryPolicy},
};
use batlehub_web::{new_access_lock, RegistryModeMap};

// ── Package explorer (explore.rs) ─────────────────────────────────────────────

/// Like `make_app` but with explore permissions open for all roles across all registries.
async fn make_explore_app(
    repo: Arc<InMemoryRepo>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    make_explore_app_with_limits(repo, None, None).await
}

/// The same, with the two `[limits]` page sizes set — each the key that decides
/// both what an unasked-for page gets and the most one request may ask for, for
/// its own list.
async fn make_explore_app_with_limits(
    repo: Arc<InMemoryRepo>,
    versions_per_page: Option<u64>,
    packages_per_page: Option<u64>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let repo_dyn: Arc<dyn PackageRepository> = repo.clone();
    let storage: Arc<dyn StorageBackend> = InMemoryStorage::new();
    let cache: Arc<dyn CacheStore> = Arc::new(InMemoryCacheStore::new());

    let reg_names = ["github", "npm", "cargo", "openvsx", "go", "vscode"];
    let registries: HashMap<String, Arc<dyn RegistryClient>> = reg_names
        .iter()
        .map(|n| {
            (
                n.to_string(),
                FixedRegistry::new(*n) as Arc<dyn RegistryClient>,
            )
        })
        .collect();
    let policies: HashMap<String, Arc<RegistryPolicy>> = reg_names
        .iter()
        .map(|n| (n.to_string(), Arc::new(rbac_policy(repo_dyn.clone()))))
        .collect();

    let local_svc = make_local_svc(storage.clone());
    // The handler reads the page size off `local_svc.hot`, which is the same
    // lock as the proxy's in production (`server/src/main.rs` clones one `Arc`)
    // and a second, independent one in this fixture. Set both, or the test would
    // be configuring a `HotConfig` nothing reads.
    {
        let mut hot = local_svc.hot.write().await;
        if let Some(per_page) = versions_per_page {
            hot.versions_per_page = per_page;
        }
        if let Some(per_page) = packages_per_page {
            hot.packages_per_page = per_page;
        }
    }
    let proxy_svc = Arc::new(ProxyService {
        hot: new_hot_lock(HotConfig {
            registries,
            policies,
            versions_per_page: versions_per_page
                .unwrap_or(batlehub_core::services::hot_config::DEFAULT_VERSIONS_PER_PAGE),
            packages_per_page: packages_per_page
                .unwrap_or(batlehub_core::services::hot_config::DEFAULT_PACKAGES_PER_PAGE),
            ..Default::default()
        }),
        storage: storage.clone(),
        cache,
        repo: repo_dyn.clone(),
        artifact_meta: NoopArtifactMeta::arc(),
        metrics: Arc::new(ProxyMetrics::new(&[])),
        sbom: None,
        readme: None,
        discovery: Default::default(),
    });
    let admin_svc = Arc::new(AdminService::new(repo_dyn));
    let token_repo: Arc<dyn UserTokenRepository> = Arc::new(NullTokenRepository);

    let regs: std::collections::HashSet<String> = reg_names.iter().map(|s| s.to_string()).collect();
    let access_config = new_access_lock(batlehub_web::AccessConfig {
        anonymous: regs.clone(),
        user: regs.clone(),
        admin: regs.clone(),
        groups: HashMap::new(),
        explore_anonymous: regs.clone(),
        explore_user: regs.clone(),
        explore_admin: regs.clone(),
    });
    let registry_map = registry_map_for(&[
        ("github", "github"),
        ("npm", "npm"),
        ("cargo", "cargo"),
        ("openvsx", "openvsx"),
        ("go", "goproxy"),
        ("vscode", "vscode-marketplace"),
    ]);
    let cargo_indexes = batlehub_web::CargoIndexMap::default();
    finish_test_app(
        proxy_svc,
        admin_svc,
        token_repo,
        access_config,
        registry_map,
        local_svc,
        RegistryModeMap::default(),
        cargo_indexes,
        ConfigureAppDefaults::default(),
        test_auth_providers(),
    )
    .await
}

#[actix_web::test]
async fn explore_packages_returns_empty_list_initially() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[actix_web::test]
async fn explore_packages_anonymous_returns_empty_with_explore_access() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

#[actix_web::test]
async fn explore_packages_with_specific_accessible_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
}

#[actix_web::test]
async fn explore_packages_inaccessible_registry_returns_empty() {
    // With make_app (empty explore sets), any registry filter returns empty
    let app = make_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[actix_web::test]
async fn explore_packages_sort_by_name() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?sort=name")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn explore_packages_sort_by_recent() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages?sort=recent")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn explore_registry_stats_returns_empty_initially() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/registries")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    // Response is now an object {registries: [], upstream_unavailable: bool}
    assert!(body["registries"].is_array());
    assert_eq!(body["upstream_unavailable"], false);
}

/// The local-row half of the detail response is byte-for-byte what it was
/// before RFC 0007, and `?upstream=skip` is how a caller asks for exactly that
/// (§9). This test used to assert the *whole* response, and the discovery read
/// is precisely the thing that changed it — so it now names the shape it is
/// pinning rather than pinning the default and calling it "unknown package".
#[actix_web::test]
async fn explore_package_detail_with_upstream_skipped_returns_only_local_rows() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/npm/lodash?upstream=skip")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["registry"], "npm");
    assert_eq!(body["name"], "lodash");
    assert_eq!(body["versions"], serde_json::json!([]));
    assert_eq!(body["upstream"]["attempted"], false);
    assert!(body["gate"]["registry_accessible"]
        .as_bool()
        .unwrap_or(false));
}

/// And the default now answers, which is the whole point of RFC 0007 §2.3: the
/// console's own search finds packages this instance holds nothing of, and the
/// page it links to used to say *"no versions yet"*.
///
/// `explore_upstream_detail.rs` is where that behaviour is tested properly;
/// this is here so the *change in default* is visible from the suite that
/// pinned the old one.
#[actix_web::test]
async fn explore_package_detail_now_answers_for_a_package_held_nowhere() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/npm/lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;

    assert_eq!(body["upstream"]["attempted"], true);
    let versions = body["versions"].as_array().unwrap();
    assert!(!versions.is_empty(), "the page said nothing: {body}");
    for version in versions {
        assert_eq!(version["source"], "upstream");
        // Nothing about a version this instance has never held is stated as a
        // number: `0` downloads would be a claim we cannot support.
        assert!(version["download_count"].is_null());
        assert_eq!(version["vulnerabilities_scanned"], false);
    }
}

#[actix_web::test]
async fn explore_package_detail_inaccessible_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/unknown-reg/some-pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert!(!body["gate"]["registry_accessible"]
        .as_bool()
        .unwrap_or(true));
}

#[actix_web::test]
async fn explore_upstream_search_returns_empty_with_no_results() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/upstream?name=lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["items"], serde_json::json!([]));
}

#[actix_web::test]
async fn explore_upstream_search_filtered_by_registry() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/upstream?name=lodash&registry=npm")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── Explore cache — response shape ───────────────────────────────────────────

#[actix_web::test]
async fn explore_packages_response_includes_upstream_unavailable_false() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["upstream_unavailable"], false);
}

#[actix_web::test]
async fn explore_registry_stats_response_has_object_shape_with_upstream_field() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/registries")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert!(body.is_object(), "response must be an object, not an array");
    assert!(body["registries"].is_array());
    assert_eq!(body["upstream_unavailable"], false);
}

#[actix_web::test]
async fn explore_package_detail_response_includes_upstream_unavailable_false() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages/npm/lodash")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert_eq!(body["upstream_unavailable"], false);
}

// ── Explore cache — invalidation endpoint ────────────────────────────────────

#[actix_web::test]
async fn explore_invalidate_requires_admin_role() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn explore_invalidate_all_returns_ok_for_admin() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn explore_invalidate_by_registry_returns_ok_for_admin() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload(r#"{"registry":"npm"}"#)
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["ok"], true);
}

#[actix_web::test]
async fn explore_invalidate_anonymous_is_rejected() {
    let app = make_explore_app(InMemoryRepo::new()).await;
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    let resp = call_service(&app, req).await;
    // anonymous has no admin role → 403
    assert_eq!(resp.status(), 403);
}

#[actix_web::test]
async fn explore_cache_serves_data_after_second_request() {
    // Verifies the cache is populated on first hit and returned on subsequent calls.
    let app = make_explore_app(InMemoryRepo::new()).await;
    for _ in 0..2 {
        let req = TestRequest::get()
            .uri("/api/v1/explore/packages")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert_eq!(resp.status(), 200);
        let body: Value = read_body_json(resp).await;
        assert_eq!(body["upstream_unavailable"], false);
    }
}

#[actix_web::test]
async fn explore_cache_clears_after_invalidate_all() {
    let app = make_explore_app(InMemoryRepo::new()).await;

    // Prime the cache
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Flush via admin endpoint
    let req = TestRequest::post()
        .uri("/api/v1/admin/explore/invalidate")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/json"))
        .set_payload("{}")
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);

    // Subsequent request should still succeed (cache refills from DB)
    let req = TestRequest::get()
        .uri("/api/v1/explore/packages")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let body: Value = read_body_json(resp).await;
    assert_eq!(body["upstream_unavailable"], false);
}

// ── One page of a long version list (RFC 0013 §4.3) ───────────────────────────
//
// The endpoint used to answer with every version it could assemble, and the
// console filtered and paged that list in the browser. Two things were wrong
// with it and only one was visible: 169 versions of
// `@babel/plugin-transform-runtime` cost a vulnerability read and an SBOM read
// each to serve a table showing 25 rows, and — once the answer is a page — a
// filter applied in the browser searches what happened to arrive rather than
// what this server has.
//
// So the filter, the pager and the pre-release toggle are query parameters, and
// the answer carries the counts the console says out loud.

/// One package, `n` held versions, zero-padded so the endpoint's
/// newest-first-by-string order is the same as newest-first by number.
async fn seed_versions(repo: &Arc<InMemoryRepo>, name: &str, n: usize) {
    for i in 0..n {
        repo.record_access(batlehub_core::entities::AccessEvent::allowed_download(
            batlehub_core::entities::PackageId::new("npm", name, format!("1.{i:03}.0")),
            Some("user-1".to_owned()),
            batlehub_core::entities::Role::User,
        ))
        .await
        .unwrap();
    }
}

async fn detail_body(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    uri: &str,
) -> Value {
    let resp = call_service(app, TestRequest::get().uri(uri).to_request()).await;
    assert_eq!(resp.status(), 200, "GET {uri}");
    read_body_json(resp).await
}

#[actix_web::test]
async fn a_long_version_list_comes_back_one_page_at_a_time() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25",
    )
    .await;

    assert_eq!(body["versions"].as_array().unwrap().len(), 25);
    assert_eq!(body["versions_page"]["page"], 0);
    assert_eq!(body["versions_page"]["per_page"], 25);
    // The totals are about the package, not about the page — they are what the
    // console's `25 of 60 shown` is made of.
    assert_eq!(body["versions_page"]["total"], 60);
    assert_eq!(body["versions_page"]["unfiltered_total"], 60);
    // Newest first, and page two picks up exactly where page one stopped.
    assert_eq!(body["versions"][0]["version"], "1.059.0");

    let second = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25&page=1",
    )
    .await;
    assert_eq!(second["versions"][0]["version"], "1.034.0");
    assert_eq!(second["versions_page"]["page"], 1);
}

/// The operator's key is the answer to "how much of a version list will this
/// server build for one request", so it is both the unasked-for default…
#[actix_web::test]
async fn the_configured_page_size_is_what_an_unasked_for_page_gets() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app_with_limits(repo, Some(10), None).await;

    let body = detail_body(&app, "/api/v1/explore/packages/npm/lodash?upstream=skip").await;

    assert_eq!(body["versions"].as_array().unwrap().len(), 10);
    assert_eq!(body["versions_page"]["per_page"], 10);
    assert_eq!(body["versions_page"]["total"], 60);
}

/// …and the ceiling. A caller asking for more gets the operator's number rather
/// than an error: the ask is not illegitimate, it is simply more than this
/// server is willing to serialise at once, and it is reported back so the caller
/// can page instead of silently missing rows.
#[actix_web::test]
async fn asking_for_more_than_the_operator_allows_gets_the_operators_number() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app_with_limits(repo, Some(10), None).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=50",
    )
    .await;

    assert_eq!(body["versions"].as_array().unwrap().len(), 10);
    assert_eq!(body["versions_page"]["per_page"], 10);
}

/// The whole reason the filter is here rather than in the console: `1.004.0` is
/// on the third page, and a filter that only searched the page it was handed
/// would answer *no* about a version this server holds.
#[actix_web::test]
async fn the_filter_searches_every_page_rather_than_the_one_returned() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25&q=004",
    )
    .await;

    assert_eq!(body["versions"].as_array().unwrap().len(), 1);
    assert_eq!(body["versions"][0]["version"], "1.004.0");
    // What the console's `1 of 60 shown` is made of: the filtered total and the
    // one before it, so a filtered list is never mistaken for a short one.
    assert_eq!(body["versions_page"]["total"], 1);
    assert_eq!(body["versions_page"]["unfiltered_total"], 60);
}

#[actix_web::test]
async fn a_filter_matching_nothing_is_an_empty_page_not_an_error() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&q=9.9.9",
    )
    .await;

    assert!(body["versions"].as_array().unwrap().is_empty());
    assert_eq!(body["versions_page"]["total"], 0);
    assert_eq!(body["versions_page"]["unfiltered_total"], 60);
}

/// A hand-edited URL, or a link sent before versions were yanked. Answering it
/// with an empty page would be indistinguishable from a package with nothing in
/// it.
#[actix_web::test]
async fn a_page_past_the_end_is_the_last_page() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25&page=99",
    )
    .await;

    assert_eq!(body["versions_page"]["page"], 2);
    assert_eq!(body["versions"].as_array().unwrap().len(), 10);
}

/// The compatible default: this endpoint has always answered with every version,
/// and dropping rows out of an existing caller's response on an upgrade is not a
/// change anyone opted into.
#[actix_web::test]
async fn pre_releases_are_in_the_answer_until_the_caller_says_otherwise() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 3).await;
    repo.record_access(batlehub_core::entities::AccessEvent::allowed_download(
        batlehub_core::entities::PackageId::new("npm", "lodash", "1.003.0-rc.1"),
        Some("user-1".to_owned()),
        batlehub_core::entities::Role::User,
    ))
    .await
    .unwrap();
    let app = make_explore_app(repo).await;

    let shown = detail_body(&app, "/api/v1/explore/packages/npm/lodash?upstream=skip").await;
    assert_eq!(shown["versions"].as_array().unwrap().len(), 4);
    assert_eq!(shown["versions_page"]["prerelease_total"], 1);
    assert_eq!(shown["versions_page"]["hidden_prereleases"], 0);

    let hidden = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&prereleases=hide",
    )
    .await;
    assert_eq!(hidden["versions"].as_array().unwrap().len(), 3);
    assert_eq!(hidden["versions_page"]["prerelease_total"], 1);
    assert_eq!(hidden["versions_page"]["hidden_prereleases"], 1);
    // The count the console offers is what is *currently* hidden, and the total
    // is what exists — two numbers, because the pinned version keeps its own.
    assert_eq!(hidden["versions_page"]["unfiltered_total"], 4);
}

/// A link to `?version=…-rc.1` must not answer with a list its own subject is
/// missing from — the console would be marking a row it was never given.
#[actix_web::test]
async fn the_version_asked_for_survives_the_pre_release_filter() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 3).await;
    repo.record_access(batlehub_core::entities::AccessEvent::allowed_download(
        batlehub_core::entities::PackageId::new("npm", "lodash", "1.003.0-rc.1"),
        Some("user-1".to_owned()),
        batlehub_core::entities::Role::User,
    ))
    .await
    .unwrap();
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&prereleases=hide&version=1.003.0-rc.1",
    )
    .await;

    let versions: Vec<&str> = body["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["version"].as_str().unwrap())
        .collect();
    assert!(versions.contains(&"1.003.0-rc.1"), "{versions:?}");
    assert_eq!(body["versions_page"]["hidden_prereleases"], 0);
    assert_eq!(body["selected_version"], "1.003.0-rc.1");
}

/// A package that has never cut a stable release: hiding every pre-release would
/// answer forty versions with an empty table, so the one the caller is about to
/// be pointed at survives too.
#[actix_web::test]
async fn a_pre_release_only_package_still_answers_with_a_row() {
    let repo = InMemoryRepo::new();
    for i in 0..3 {
        repo.record_access(batlehub_core::entities::AccessEvent::allowed_download(
            batlehub_core::entities::PackageId::new("npm", "alpha", format!("0.{i}.0-rc.1")),
            Some("user-1".to_owned()),
            batlehub_core::entities::Role::User,
        ))
        .await
        .unwrap();
    }
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/alpha?upstream=skip&prereleases=hide",
    )
    .await;

    assert_eq!(body["default_version"], "0.2.0-rc.1");
    assert_eq!(body["versions"].as_array().unwrap().len(), 1);
    assert_eq!(body["versions"][0]["version"], "0.2.0-rc.1");
}

/// A link to a version sixty rows down opens on the page that holds it. Only
/// this side can say which page that is, and the answer says which it was.
#[actix_web::test]
async fn a_version_with_no_page_asked_for_opens_on_its_own_page() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 60).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25&version=1.004.0",
    )
    .await;

    assert_eq!(body["versions_page"]["page"], 2);
    assert_eq!(body["selected_version"], "1.004.0");

    // An explicit page outranks it: the caller turned to page one *while* that
    // version was selected, and a server that pulled them back would be
    // overruling the address they are looking at.
    let pinned = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&per_page=25&version=1.004.0&page=0",
    )
    .await;
    assert_eq!(pinned["versions_page"]["page"], 0);
}

/// A typo, or a version yanked since the link was sent. The caller cannot tell
/// that from "on another page", so this side answers it.
#[actix_web::test]
async fn a_version_this_package_does_not_have_is_not_echoed_back() {
    let repo = InMemoryRepo::new();
    seed_versions(&repo, "lodash", 3).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(
        &app,
        "/api/v1/explore/packages/npm/lodash?upstream=skip&version=9.9.9",
    )
    .await;

    assert!(body["selected_version"].is_null());
    assert_eq!(body["default_version"], "1.002.0");
}

// ── One page of the catalog (RFC 0013 §4.3) ───────────────────────────────────
//
// The catalog has always been paginated; what it did not have was an operator's
// say in how long a page is. 20 was a literal in two places — a `serde` default
// here and a `const perPage` in the console — so the number could not be
// changed without a rebuild, and the two copies could disagree.

/// `n` distinct package names in one registry, so the listing has something to
/// page through.
async fn seed_packages(repo: &Arc<InMemoryRepo>, n: usize) {
    for i in 0..n {
        repo.record_access(batlehub_core::entities::AccessEvent::allowed_download(
            batlehub_core::entities::PackageId::new("npm", format!("pkg-{i:03}"), "1.0.0"),
            Some("user-1".to_owned()),
            batlehub_core::entities::Role::User,
        ))
        .await
        .unwrap();
    }
}

#[actix_web::test]
async fn the_catalog_answers_twenty_packages_by_default() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(&app, "/api/v1/explore/packages").await;

    assert_eq!(body["items"].as_array().unwrap().len(), 20);
    assert_eq!(body["per_page"], 20);
    assert_eq!(body["total"], 25);
}

#[actix_web::test]
async fn the_configured_catalog_page_size_is_what_an_unasked_for_page_gets() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app_with_limits(repo, None, Some(5)).await;

    let body = detail_body(&app, "/api/v1/explore/packages").await;

    assert_eq!(body["items"].as_array().unwrap().len(), 5);
    assert_eq!(body["per_page"], 5);
    // The total is still about the catalog rather than about the page, which is
    // what the console's pager divides.
    assert_eq!(body["total"], 25);
}

/// The same two readings as the version list: default *and* ceiling.
#[actix_web::test]
async fn asking_for_more_of_the_catalog_than_the_operator_allows_gets_the_operators_number() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app_with_limits(repo, None, Some(5)).await;

    let body = detail_body(&app, "/api/v1/explore/packages?per_page=50").await;

    assert_eq!(body["items"].as_array().unwrap().len(), 5);
    assert_eq!(body["per_page"], 5);
}

/// Asking for *less* is honoured — the ceiling is a ceiling, not a fixed size.
#[actix_web::test]
async fn a_caller_may_still_ask_the_catalog_for_a_shorter_page() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(&app, "/api/v1/explore/packages?per_page=3").await;

    assert_eq!(body["items"].as_array().unwrap().len(), 3);
    assert_eq!(body["per_page"], 3);
}

/// `per_page=0` collapsed the listing query's cache key onto the count query's
/// — both become `limit=0, offset=0` — so it has always been clamped rather
/// than passed through. The configured ceiling must not have lost that.
#[actix_web::test]
async fn a_zero_page_size_is_still_clamped_to_one_row() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app(repo).await;

    let body = detail_body(&app, "/api/v1/explore/packages?per_page=0").await;

    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["per_page"], 1);
}

/// The two keys are one each: sizing the version table must not resize the
/// catalog, and the other way round.
#[actix_web::test]
async fn the_two_page_sizes_are_independent() {
    let repo = InMemoryRepo::new();
    seed_packages(&repo, 25).await;
    let app = make_explore_app_with_limits(repo, Some(3), None).await;

    let body = detail_body(&app, "/api/v1/explore/packages").await;

    assert_eq!(body["per_page"], 20, "versions_per_page moved the catalog");
}
