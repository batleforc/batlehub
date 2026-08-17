//! Search, across the five ecosystems that share one path (RFC 0009 §7.7).
//!
//! Before this phase, search was five separate non-answers. NuGet's `/v3/query`
//! returned a hardcoded `{"totalHits": 0, "data": []}` in proxy and hybrid mode
//! while the service index advertised `SearchQueryService` pointing at it — so
//! `dotnet package search` reported nothing against a registry holding
//! thousands of packages, and every signal a test reads was green. `vsx`
//! free-text did the same. npm, cargo and Composer had no route at all.
//!
//! The rung that makes the rest defensible is the last one: when the upstream is
//! unreachable, search answers from the packages this registry already holds.
//! Not an error, and not an empty list — what we actually have.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, TestRequest};
use serde_json::Value;

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;

async fn get_json<S>(app: &S, uri: &str) -> (Value, String)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200, "{uri} should be served");
    let cache = resp
        .headers()
        .get("X-BatleHub-Cache")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let body = actix_web::test::read_body(resp).await;
    (serde_json::from_slice(&body).expect("JSON"), cache)
}

/// The defect this phase exists for: the endpoint answered `200` with nothing
/// in it, forever, while the service index promised it worked.
#[actix_web::test]
async fn nuget_search_is_no_longer_a_stub() {
    let app = make_app(InMemoryRepo::new()).await;
    let (doc, _) = get_json(&app, "/proxy/nuget/nuget/v3/query?q=fixed").await;

    assert!(
        doc["totalHits"].as_u64().unwrap_or(0) > 0,
        "search reported nothing for a query the upstream answers: {doc}"
    );
    let names: Vec<&str> = doc["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    assert!(names.contains(&"fixed-alpha"), "got {names:?}");
}

#[actix_web::test]
async fn npm_search_renders_the_npm_shape() {
    let app = make_app(InMemoryRepo::new()).await;
    let (doc, _) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed").await;

    let names: Vec<&str> = doc["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["package"]["name"].as_str())
        .collect();
    assert!(names.contains(&"fixed-alpha"), "got {names:?}");
    assert_eq!(doc["total"].as_u64().unwrap_or(0) as usize, names.len());

    // "the npm shape" was, until `tests/heavy/npm.sh` ran a real client against
    // it, the shape this test asserted rather than the one npm reads. npm maps
    // over `maintainers` without a guard, so a hit without the field ends
    // `npm search` in "Cannot read properties of undefined (reading 'map')" —
    // a crash, from a `200` whose names this test was checking and finding
    // right (RFC 0009 §12.16).
    for hit in doc["objects"].as_array().unwrap() {
        assert!(
            hit["package"]["maintainers"].is_array(),
            "every hit needs a maintainers array — npm dereferences it: {hit}"
        );
    }
}

#[actix_web::test]
async fn cargo_search_renders_the_cargo_shape() {
    let app = make_app(InMemoryRepo::new()).await;
    let (doc, _) = get_json(&app, "/proxy/cargo/api/v1/crates?q=fixed").await;

    let names: Vec<&str> = doc["crates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(names.contains(&"fixed-alpha"), "got {names:?}");
    assert!(doc["meta"]["total"].as_u64().unwrap_or(0) > 0);
}

/// A search result names a version, and there is no list here to repair it
/// against — so a hit whose named version is blocked is dropped, and the total
/// moves with it. A total left unadjusted is its own bug: clients paginate by
/// offset, so the next page would skip whatever the removal shifted.
///
/// Two apps rather than one. Search filters against the registry-wide blocked
/// snapshot, which has a 30-second TTL — so reading the baseline from the app
/// under test warms that snapshot with an empty set and the block does not land
/// (the same trade conda's `repodata.json` and RubyGems' `/versions` make).
#[actix_web::test]
async fn a_blocked_version_is_dropped_from_results_and_from_the_total() {
    let unblocked = make_app(InMemoryRepo::new()).await;
    let (before, _) = get_json(&unblocked, "/proxy/npm/-/v1/search?text=fixed").await;
    let before_total = before["total"].as_u64().unwrap();
    assert!(before_total >= 2, "fixture should return at least two hits");

    let app = make_app(InMemoryRepo::new()).await;
    // `fixed-alpha` is returned at 1.1.0 by the fixture upstream.
    block_version(&app, "npm", "fixed-alpha", "1.1.0").await;

    let (after, _) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed").await;
    let names: Vec<&str> = after["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["package"]["name"].as_str())
        .collect();
    assert!(
        !names.contains(&"fixed-alpha"),
        "a blocked version was still offered: {names:?}"
    );
    assert_eq!(
        after["total"].as_u64().unwrap() as usize,
        names.len(),
        "the total must be adjusted, or offset pagination skips a result"
    );
    assert!(
        (after["total"].as_u64().unwrap()) < before_total,
        "the total must shrink with the result set"
    );
}

/// Rung 1: a repeated query is answered from the cache rather than asked again.
#[actix_web::test]
async fn a_repeated_query_is_served_from_cache() {
    let app = make_app(InMemoryRepo::new()).await;

    let (_, first) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed").await;
    assert_eq!(first, "miss", "the first query goes upstream");

    let (_, second) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed").await;
    assert_eq!(second, "hit", "the second must not ask upstream again");
}

/// Composer discovers `search.json` and `list.json` only from URL templates in
/// `packages.json`. Omitting them makes both routes unreachable however
/// correctly they are implemented — which is how phase 6 and phase 7 shipped
/// them, and what Composer 2.10.2 showed (RFC 0009 §12.5).
#[actix_web::test]
async fn composer_packages_json_advertises_the_endpoints_it_serves() {
    let app = make_app(InMemoryRepo::new()).await;
    let (doc, _) = get_json(&app, "/proxy/composer/packages.json").await;

    for key in ["metadata-url", "search", "list"] {
        assert!(
            doc.get(key).and_then(|v| v.as_str()).is_some(),
            "packages.json must advertise {key:?}, or the client never asks: {doc}"
        );
    }
    assert!(
        doc["search"].as_str().unwrap().contains("%query%"),
        "the search template must carry Composer's %query% placeholder"
    );
}

/// Paging is into the result set, not into a page already truncated to the page
/// size — RFC 0009 §12.16.
///
/// §12.4 measured `dotnet package search` sending `skip=0&take=20` then
/// `skip=20`, and the fix parsed `skip` and applied it *after* the search had
/// been limited to `take` hits. So the second page was always empty: the client
/// asks for hits 20-39 of a registry that was only asked for its first 20.
/// The same shape on the npm side, where `from` was not read at all.
#[actix_web::test]
async fn nuget_search_second_page_is_not_empty() {
    let app = make_app(InMemoryRepo::new()).await;

    let (all, _) = get_json(&app, "/proxy/nuget/nuget/v3/query?q=fixed&take=10").await;
    let total = all["data"].as_array().unwrap().len();
    assert!(
        total >= 2,
        "this test needs at least two matches, got {total}"
    );

    let (first, _) = get_json(&app, "/proxy/nuget/nuget/v3/query?q=fixed&take=1&skip=0").await;
    let (second, _) = get_json(&app, "/proxy/nuget/nuget/v3/query?q=fixed&take=1&skip=1").await;

    let id = |doc: &serde_json::Value| doc["data"][0]["id"].as_str().unwrap_or_default().to_owned();
    assert!(!id(&first).is_empty(), "page 1 is empty: {first}");
    assert!(
        !id(&second).is_empty(),
        "page 2 is empty — skip is being applied to a page that was already cut to `take`: {second}"
    );
    assert_ne!(id(&first), id(&second), "both pages returned the same hit");
}

#[actix_web::test]
async fn npm_search_honours_from() {
    let app = make_app(InMemoryRepo::new()).await;

    let (all, _) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed&size=10").await;
    assert!(
        all["objects"].as_array().unwrap().len() >= 2,
        "this test needs at least two matches"
    );

    let (first, _) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed&size=1&from=0").await;
    let (second, _) = get_json(&app, "/proxy/npm/-/v1/search?text=fixed&size=1&from=1").await;

    let name = |doc: &serde_json::Value| {
        doc["objects"][0]["package"]["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    assert!(!name(&second).is_empty(), "page 2 is empty: {second}");
    assert_ne!(
        name(&first),
        name(&second),
        "both pages returned the same hit"
    );
}
