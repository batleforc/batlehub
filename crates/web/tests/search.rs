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
