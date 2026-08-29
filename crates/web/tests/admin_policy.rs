//! The `policy` table's admin API — RFC 0015 §6.3.
//!
//! §4.1 is why this is an API rather than a config block: *"a registry with
//! 200 000 packages will not enumerate them in TOML, let alone their two million
//! versions"*. The store's own properties are asserted in
//! `crates/adapters/tests/pg_policy.rs`, against both implementations; what is
//! left here is the HTTP surface — the tier a route writes, what it refuses, and
//! who may reach it.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::{json, Value};

use batlehub_adapters::in_memory::InMemoryPackageRepository as InMemoryRepo;

const PKG: &str = "/api/v1/admin/registries/npm/policy/package/lodash";
const VER: &str = "/api/v1/admin/registries/npm/policy/version/lodash/1.0.0";

async fn put<S: TestService>(app: &S, uri: &str, token: &str, body: Value) -> u16 {
    let req = TestRequest::put()
        .uri(uri)
        .insert_header(("Authorization", bearer(token)))
        .set_json(body)
        .to_request();
    call_service(app, req).await.status().as_u16()
}

async fn get<S: TestService>(app: &S, uri: &str, token: &str) -> (u16, Value) {
    let req = TestRequest::get()
        .uri(uri)
        .insert_header(("Authorization", bearer(token)))
        .to_request();
    let resp = call_service(app, req).await;
    let status = resp.status().as_u16();
    if status != 200 {
        return (status, Value::Null);
    }
    (status, read_body_json(resp).await)
}

/// A package policy round-trips through the API.
#[actix_web::test]
async fn a_package_policy_round_trips() {
    let app = make_app(InMemoryRepo::new()).await;

    assert_eq!(
        put(
            &app,
            PKG,
            ADMIN_TOKEN,
            json!({
                "visibility": "team",
                "versioning": { "enforce_semver": true, "immutable": "released", "monotonic": true },
                "quota": { "max_bytes_per_user": 1024, "block": true },
                "rules": [{ "gate": "release_age_gate", "settings": { "min_age_secs": 0 } }],
            })
        )
        .await,
        204
    );

    let (status, body) = get(&app, PKG, ADMIN_TOKEN).await;
    assert_eq!(status, 200);
    assert_eq!(body["visibility"], "team");
    assert_eq!(body["versioning"]["immutable"], "released");
    assert!(body["versioning"]["monotonic"].as_bool().unwrap());
    assert_eq!(body["quota"]["max_bytes_per_user"], 1024);
    assert_eq!(body["rules"][0]["gate"], "release_age_gate");
}

/// An absent field means **inherit**, and must come back absent.
///
/// The distinction §4.3 makes for grants, in its policy form: a response that
/// filled in `"visibility": "public"` where nothing was written would tell a
/// client the node overrides its namespace when it does not.
#[actix_web::test]
async fn an_absent_field_is_absent_in_the_response() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(&app, PKG, ADMIN_TOKEN, json!({ "visibility": "team" })).await,
        204
    );

    let (_, body) = get(&app, PKG, ADMIN_TOKEN).await;
    assert_eq!(body["visibility"], "team");
    assert!(
        body.get("versioning").is_none(),
        "an unset policy must not be reported as a default one: {body}"
    );
    assert!(body.get("quota").is_none(), "{body}");
    assert!(body.get("rules").is_none(), "{body}");
}

/// An unwritten node is `404`, not an empty document.
///
/// An empty `PolicyDto` is a legal thing to *send* — it clears the node — so
/// answering one for an absent node would make "nothing is written here" and
/// "everything here is cleared" the same response.
#[actix_web::test]
async fn an_unwritten_node_is_404() {
    let app = make_app(InMemoryRepo::new()).await;
    let (status, _) = get(&app, PKG, ADMIN_TOKEN).await;
    assert_eq!(status, 404);
}

/// The version route writes the version tier, not the package tier.
///
/// The reason §6.3's routes name the tier rather than inferring it: a caller who
/// omitted a version would otherwise silently write one level shallower, and the
/// override would apply to every version of the package instead of the one they
/// meant.
#[actix_web::test]
async fn the_version_route_writes_the_version_tier() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(
            &app,
            VER,
            ADMIN_TOKEN,
            json!({ "versioning": { "immutable": "always" } })
        )
        .await,
        204
    );

    let (status, body) = get(&app, VER, ADMIN_TOKEN).await;
    assert_eq!(status, 200);
    assert_eq!(body["versioning"]["immutable"], "always");

    // …and the package tier is untouched.
    let (pkg_status, _) = get(&app, PKG, ADMIN_TOKEN).await;
    assert_eq!(
        pkg_status, 404,
        "writing a version must not create a package-tier node"
    );
}

/// §4.1's tier rules reach the API as a `400`.
///
/// The naming half of `versioning` governs what a version may be *called*, and
/// at version tier the name already exists — `enforce_semver` on `1.0.0` has
/// nothing left to decide. Rejected rather than silently ignored, which is what
/// §4.1 asks for.
#[actix_web::test]
async fn a_naming_field_at_version_tier_is_refused() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(
            &app,
            VER,
            ADMIN_TOKEN,
            json!({ "versioning": { "enforce_semver": true } })
        )
        .await,
        400
    );
    assert_eq!(
        put(
            &app,
            VER,
            ADMIN_TOKEN,
            json!({ "quota": { "block": true } })
        )
        .await,
        400,
        "a per-version quota limits a thing published exactly once"
    );
}

/// …and the same fields are fine one tier up, which makes the test above about
/// the tier rather than about the fields.
#[actix_web::test]
async fn the_same_fields_are_accepted_at_package_tier() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(
            &app,
            PKG,
            ADMIN_TOKEN,
            json!({
                "versioning": { "enforce_semver": true },
                "quota": { "block": true },
            })
        )
        .await,
        204
    );
}

/// An empty body clears the node rather than storing an override that overrides
/// nothing.
#[actix_web::test]
async fn an_empty_body_clears_the_node() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(&app, PKG, ADMIN_TOKEN, json!({ "visibility": "team" })).await,
        204
    );
    assert_eq!(get(&app, PKG, ADMIN_TOKEN).await.0, 200);

    assert_eq!(put(&app, PKG, ADMIN_TOKEN, json!({})).await, 204);
    assert_eq!(get(&app, PKG, ADMIN_TOKEN).await.0, 404);
}

/// DELETE clears the node, and an absent one is not an error.
#[actix_web::test]
async fn delete_clears_the_node_and_absent_is_not_an_error() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(&app, PKG, ADMIN_TOKEN, json!({ "visibility": "team" })).await,
        204
    );

    let del = |uri: &'static str| {
        let app = &app;
        async move {
            let req = TestRequest::delete()
                .uri(uri)
                .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
                .to_request();
            call_service(app, req).await.status().as_u16()
        }
    };
    assert_eq!(del(PKG).await, 204);
    assert_eq!(get(&app, PKG, ADMIN_TOKEN).await.0, 404);
    assert_eq!(del(PKG).await, 204, "absent is not an error");
}

/// Every route is admin-only.
///
/// Asserted per route rather than once, because these six are registered as two
/// families of three and a family that missed its guard would still pass a
/// spot-check on the other.
#[actix_web::test]
async fn every_policy_route_requires_admin() {
    let app = make_app(InMemoryRepo::new()).await;

    for uri in [PKG, VER] {
        assert_eq!(
            get(&app, uri, USER_TOKEN).await.0,
            403,
            "GET {uri} must refuse a non-admin"
        );
        assert_eq!(
            put(&app, uri, USER_TOKEN, json!({ "visibility": "team" })).await,
            403,
            "PUT {uri} must refuse a non-admin"
        );
        let req = TestRequest::delete()
            .uri(uri)
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request();
        assert_eq!(
            call_service(&app, req).await.status().as_u16(),
            403,
            "DELETE {uri} must refuse a non-admin"
        );
    }
}

/// A traversal in the coordinate is a `400`, not a policy on a node the caller
/// did not name.
///
/// `node_key` is built from these path segments, so this is the same edge
/// validation every publish handler runs and for the same reason.
#[actix_web::test]
async fn a_traversal_in_the_coordinate_is_refused() {
    let app = make_app(InMemoryRepo::new()).await;

    let status = put(
        &app,
        "/api/v1/admin/registries/npm/policy/version/lodash/..%2F..%2Fetc",
        ADMIN_TOKEN,
        json!({ "versioning": { "immutable": "always" } }),
    )
    .await;
    assert!(
        status == 400 || status == 404,
        "a traversal must not be written as a node key, got {status}"
    );
}

/// The two route families do not shadow one another.
///
/// Both end in a wildcard, and a package route registered first would swallow
/// `…/policy/version/lodash/1.0.0` as a package named `version/lodash/1.0.0` —
/// which would write the wrong tier and answer `204` while doing it.
#[actix_web::test]
async fn the_version_route_is_not_swallowed_by_the_package_wildcard() {
    let app = make_app(InMemoryRepo::new()).await;
    assert_eq!(
        put(
            &app,
            VER,
            ADMIN_TOKEN,
            json!({ "versioning": { "immutable": "always" } })
        )
        .await,
        204
    );

    // If the package wildcard had matched, the node would be readable as a
    // package called `version/lodash/1.0.0`.
    let (status, _) = get(
        &app,
        "/api/v1/admin/registries/npm/policy/package/version/lodash/1.0.0",
        ADMIN_TOKEN,
    )
    .await;
    assert_eq!(
        status, 404,
        "the version write must not have landed on the package tier"
    );
}
