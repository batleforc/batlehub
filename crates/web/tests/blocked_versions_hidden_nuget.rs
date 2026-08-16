//! Blocking a NuGet version hides it from the flat index, not just from the
//! download.
//!
//! `dotnet restore` resolves a version range against
//! `/v3/flat/{id}/index.json`. Leaving a blocked version in that document makes
//! the restore choose it and then be refused the `.nupkg` — the block reads as
//! a broken feed rather than as policy.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::Value;

fn nuget_app_parts(mode: RegistryMode) -> LocalRegistryAppParts {
    local_registry_app_parts("local-nuget", "nuget", mode, None)
}

fn flat_index_uri(package: &str) -> String {
    format!("/proxy/local-nuget/nuget/v3/flat/{package}/index.json")
}

async fn versions_in<S>(app: &S, package: &str) -> Vec<String>
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&flat_index_uri(package))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let doc: Value = read_body_json(call_service(app, req).await).await;
    doc["versions"]
        .as_array()
        .expect("flat index has a versions array")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect()
}

#[actix_web::test]
async fn proxy_flat_index_hides_a_blocked_version() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    // This first read also warms the document cache: the block below has to take
    // effect on the very next request, which only holds because what is cached
    // is the raw upstream document with filtering applied on the way out.
    assert_eq!(
        versions_in(&app, "newtonsoft.json").await,
        ["1.0.0", "1.1.0", "2.0.0-beta.1"]
    );

    block_version(&app, "local-nuget", "newtonsoft.json", "1.1.0").await;

    assert_eq!(
        versions_in(&app, "newtonsoft.json").await,
        ["1.0.0", "2.0.0-beta.1"],
        "the blocked version is gone and the ascending order survives"
    );
}

/// NuGet folds `1.1.0.0` to `1.1.0`. A block recorded in the four-part spelling
/// has to hide the three-part listing, or it silently hides nothing — the
/// failure mode with no other symptom.
#[actix_web::test]
async fn proxy_flat_index_matches_across_nuget_version_spellings() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    block_version(&app, "local-nuget", "newtonsoft.json", "1.1.0.0").await;

    assert!(
        !versions_in(&app, "newtonsoft.json")
            .await
            .contains(&"1.1.0".to_owned()),
        "the four-part block did not match the three-part listing"
    );
}

/// Before this route moved off `proxy_stream` it answered with whatever
/// `fetch_artifact` returned for the `__index__` sentinel, as
/// `application/octet-stream`.
#[actix_web::test]
async fn proxy_flat_index_is_json() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    let req = TestRequest::get()
        .uri(&flat_index_uri("newtonsoft.json"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;

    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .expect("content-type set")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(ct.starts_with("application/json"), "content-type was {ct}");
}

#[actix_web::test]
async fn proxy_flat_index_of_another_package_is_untouched() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    block_version(&app, "local-nuget", "newtonsoft.json", "1.1.0").await;

    assert!(versions_in(&app, "serilog")
        .await
        .contains(&"1.1.0".to_owned()));
}

/// The second of NuGet's two listing documents. `dotnet restore` resolves
/// against the flat index, but a UI and `dotnet list package` read the
/// registration pages, so a version left there is advertised as installable
/// when it is not.
#[actix_web::test]
async fn proxy_registration_hides_a_blocked_version_and_repairs_the_page_bounds() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    block_version(&app, "local-nuget", "newtonsoft.json", "2.0.0-beta.1").await;

    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/registration5/newtonsoft.json/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let doc: Value = read_body_json(call_service(&app, req).await).await;

    let page = &doc["items"][0];
    let versions: Vec<&str> = page["items"]
        .as_array()
        .expect("registration leaves")
        .iter()
        .map(|l| l["catalogEntry"]["version"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(versions, ["1.0.0", "1.1.0"]);
    assert_eq!(page["count"], 2, "the page count must follow its leaves");
    assert_eq!(
        page["upper"], "1.1.0",
        "a client that trusts `upper` to skip a page would skip a version it may have"
    );
}

/// The two documents are keyed separately in the metadata cache. Keyed
/// together, whichever one warmed the entry would be served under the other's
/// URL.
#[actix_web::test]
async fn the_flat_index_and_registration_do_not_collide_in_the_cache() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    // Warm the registration slot first, then ask for the flat index.
    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/registration5/newtonsoft.json/index.json")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let _: Value = read_body_json(call_service(&app, req).await).await;

    assert_eq!(
        versions_in(&app, "newtonsoft.json").await,
        ["1.0.0", "1.1.0", "2.0.0-beta.1"],
        "the flat index came back as something else"
    );
}

/// Hiding governs resolution, not diagnosis: a client that pins the blocked
/// version still gets the operator's `403` and reason rather than a `404` that
/// looks like the version never existed.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = build_local_registry_app(
        nuget_app_parts(RegistryMode::Proxy),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    block_version(&app, "local-nuget", "newtonsoft.json", "1.1.0").await;

    let req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/flat/newtonsoft.json/1.1.0/newtonsoft.json.1.1.0.nupkg")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
