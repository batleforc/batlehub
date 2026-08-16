//! Blocking a conda package hides it from the channel's `repodata.json`.
//!
//! The only listing here that describes **many packages at once**, which
//! changes two things. The blocked set is a whole registry's worth rather than
//! one package's, and it comes from a 30-second snapshot rather than a
//! per-request query — `repodata.json` for a busy channel is tens of megabytes
//! and is fetched on every `conda install`, so re-querying per request would
//! put the entire block list on that path.
//!
//! That snapshot is the one place in RFC 0006 where a block is not effective on
//! the very next request, and it is load-bearing enough to be pinned rather
//! than left implicit — see `a_block_does_not_reach_an_already_warm_snapshot`.
//! Every other test here starts with a cold snapshot, so it must not read the
//! channel before blocking.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use batlehub_config::schema::RegistryMode;
use serde_json::Value;

async fn app() -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    build_local_registry_app(
        local_registry_app_parts("local-conda", "conda", RegistryMode::Proxy, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

async fn repodata<S>(app: &S, document: &str) -> Value
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let req = TestRequest::get()
        .uri(&format!("/proxy/local-conda/linux-64/{document}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    read_body_json(call_service(app, req).await).await
}

fn filenames(doc: &Value, key: &str) -> Vec<String> {
    let mut names: Vec<String> = doc[key]
        .as_object()
        .unwrap_or(&serde_json::Map::new())
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

/// A channel serves both package generations for the same release, so a block
/// that only reached one of them would leave the version installable.
#[actix_web::test]
async fn proxy_repodata_hides_a_blocked_package_from_both_generations() {
    let app = app().await;

    // Deliberately no read before the block: the first read warms the 30-second
    // snapshot with an empty set, which is exactly what the test below pins.
    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let after = repodata(&app, "repodata.json").await;
    assert_eq!(
        filenames(&after, "packages"),
        ["numpy-1.0.0-py311_0.tar.bz2", "scipy-1.1.0-py311_0.tar.bz2"]
    );
    assert!(
        filenames(&after, "packages.conda").is_empty(),
        "the `.conda` generation of the same release must go too"
    );
}

/// The blocked set spans the whole channel, so a block has to match on the
/// *pair*: another package at the same version stays.
#[actix_web::test]
async fn a_block_is_scoped_to_its_package_not_to_the_version_string() {
    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    assert!(
        filenames(&repodata(&app, "repodata.json").await, "packages")
            .contains(&"scipy-1.1.0-py311_0.tar.bz2".to_owned()),
        "scipy's 1.1.0 is a different package and must survive"
    );
}

/// `current_repodata.json` is a second document for the same channel, keyed
/// separately in the metadata cache, and filtered the same way.
#[actix_web::test]
async fn proxy_current_repodata_is_filtered_too() {
    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    assert!(
        !filenames(&repodata(&app, "current_repodata.json").await, "packages")
            .contains(&"numpy-1.1.0-py311_0.tar.bz2".to_owned())
    );
}

#[actix_web::test]
async fn the_channel_envelope_survives_filtering() {
    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let after = repodata(&app, "repodata.json").await;
    assert_eq!(after["info"]["subdir"], "linux-64");
    assert_eq!(after["repodata_version"], 1);
}

#[actix_web::test]
async fn a_channel_with_nothing_blocked_is_served_whole() {
    let app = app().await;

    let doc = repodata(&app, "repodata.json").await;
    assert_eq!(filenames(&doc, "packages").len(), 3);
    assert_eq!(filenames(&doc, "packages.conda").len(), 1);
}

/// The one asymmetry in RFC 0006, pinned rather than left to be discovered.
///
/// Conda's blocked set is a whole channel's, and `repodata.json` is fetched on
/// every `conda install`, so it is read from a snapshot refreshed every 30
/// seconds instead of queried per request. A request that warms the snapshot
/// before a block is made therefore keeps serving the unfiltered channel until
/// the snapshot expires.
///
/// The trade is deliberate — the alternative puts the whole channel's block
/// list on the hottest path in the ecosystem — and it is why the admin guide
/// states the delay. **The download gate is not delayed**: the `403` is
/// immediate, which is what keeps the blocked bytes unreachable meanwhile.
#[actix_web::test]
async fn a_block_does_not_reach_an_already_warm_snapshot() {
    let app = app().await;

    // Warm it.
    assert_eq!(
        filenames(&repodata(&app, "repodata.json").await, "packages").len(),
        3
    );

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    assert!(
        filenames(&repodata(&app, "repodata.json").await, "packages")
            .contains(&"numpy-1.1.0-py311_0.tar.bz2".to_owned()),
        "the listing lags by up to the snapshot TTL — if this ever stops being \
         true, the admin guide's stated delay is wrong and should be removed"
    );

    let req = TestRequest::get()
        .uri("/proxy/local-conda/linux-64/numpy-1.1.0-py311_0.tar.bz2")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status(),
        403,
        "the download gate reads the blocked set per request and must not lag"
    );
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_download_of_a_blocked_package_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let req = TestRequest::get()
        .uri("/proxy/local-conda/linux-64/numpy-1.1.0-py311_0.tar.bz2")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}
