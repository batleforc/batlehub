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

// ── Compressed encodings and the channel summary (RFC 0009 §7.5) ─────────────
//
// conda 23.x and mamba request `repodata.json.zst` first and fall back on 404.
// Until this phase the `{filename}` route regex admitted only `.tar.bz2`/
// `.conda`, so a `.zst` request reached no handler at all and every client paid
// the full uncompressed transfer of a document fetched on every solve.
//
// The filter runs on the JSON and compression happens after it, so there is no
// second filter to keep in step — only a second encoding of the first one's
// output. These assert exactly that: what comes back out of the decompressor is
// the *filtered* channel, not the upstream one.

async fn get_bytes<S>(app: &S, uri: &str) -> Vec<u8>
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
    actix_web::test::read_body(resp).await.to_vec()
}

/// The filtered channel, only zstd-encoded. `scipy-1.1.0` survives: the block
/// is on the *pair* `numpy 1.1.0`, not on the version string.
#[actix_web::test]
async fn zstd_repodata_decompresses_to_the_filtered_channel() {
    let app = app().await;

    // No read before the block, or the 30-second snapshot warms empty.
    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let compressed = get_bytes(&app, "/proxy/local-conda/linux-64/repodata.json.zst").await;
    let raw = zstd::decode_all(compressed.as_slice()).expect("valid zstd");
    let doc: Value = serde_json::from_slice(&raw).expect("valid JSON inside");

    assert_eq!(
        filenames(&doc, "packages"),
        ["numpy-1.0.0-py311_0.tar.bz2", "scipy-1.1.0-py311_0.tar.bz2"]
    );
    assert!(
        filenames(&doc, "packages.conda").is_empty(),
        "the `.conda` generation of the blocked release must go too"
    );
}

#[actix_web::test]
async fn bzip2_repodata_decompresses_to_the_filtered_channel() {
    use std::io::Read;

    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let compressed = get_bytes(&app, "/proxy/local-conda/linux-64/repodata.json.bz2").await;
    let mut raw = Vec::new();
    bzip2::read::BzDecoder::new(compressed.as_slice())
        .read_to_end(&mut raw)
        .expect("valid bzip2");
    let doc: Value = serde_json::from_slice(&raw).expect("valid JSON inside");

    assert_eq!(
        filenames(&doc, "packages"),
        ["numpy-1.0.0-py311_0.tar.bz2", "scipy-1.1.0-py311_0.tar.bz2"]
    );
}

/// Compressed output is cached, because recompressing tens of megabytes per
/// request is not affordable. Caching the *filtered* document is forbidden for
/// the opposite reason — it would keep serving a version after a block. Both
/// are avoided by keying on the blocked-set fingerprint.
///
/// Two apps rather than one: reading the baseline from the app under test would
/// warm its snapshot with an empty set, and the block would not land at all —
/// which measures the documented lag instead of the cache key.
#[actix_web::test]
async fn the_compressed_cache_key_depends_on_the_blocked_set() {
    let unblocked = app().await;
    let before = get_bytes(&unblocked, "/proxy/local-conda/linux-64/repodata.json.zst").await;

    let blocked = app().await;
    block_version(&blocked, "local-conda", "numpy", "1.1.0").await;
    let after = get_bytes(&blocked, "/proxy/local-conda/linux-64/repodata.json.zst").await;

    assert_ne!(
        before, after,
        "the compressed entry does not vary with the blocked set, so a block \
         would be served from a pre-block compressed copy until its TTL expired"
    );

    let before_doc: Value =
        serde_json::from_slice(&zstd::decode_all(before.as_slice()).unwrap()).expect("valid JSON");
    assert!(
        filenames(&before_doc, "packages").contains(&"numpy-1.1.0-py311_0.tar.bz2".to_owned()),
        "the unblocked baseline really did contain it"
    );
}

#[actix_web::test]
async fn channeldata_drops_a_package_whose_named_release_is_blocked() {
    let app = app().await;

    block_version(&app, "local-conda", "numpy", "1.1.0").await;

    let raw = get_bytes(&app, "/proxy/local-conda/channeldata.json").await;
    let doc: Value = serde_json::from_slice(&raw).unwrap();
    let packages = doc["packages"].as_object().expect("packages object");

    assert!(
        !packages.contains_key("numpy"),
        "channeldata names one version and has no list to repair from, so a \
         blocked newest release drops the entry"
    );
    assert!(
        packages.contains_key("scipy"),
        "an unrelated package is untouched"
    );
}

/// conda probes an index with **`HEAD`** before fetching it, and actix does not
/// route `HEAD` to a `GET` handler — so a `GET`-only route makes the probe see a
/// bodyless `404` and conclude the document does not exist.
///
/// That is how phase 3's compressed repodata shipped unreachable: the `.zst`
/// route existed, `curl -X GET` served it, and a real conda client never asked
/// for it because its `HEAD` was rejected before the handler ran. Measured
/// against micromamba 2.9.0 (RFC 0009 §12.4).
#[actix_web::test]
async fn conda_index_documents_answer_head_not_just_get() {
    let app = app().await;

    for path in [
        "/proxy/local-conda/linux-64/repodata.json",
        "/proxy/local-conda/linux-64/repodata.json.zst",
        "/proxy/local-conda/linux-64/repodata.json.bz2",
        "/proxy/local-conda/linux-64/current_repodata.json",
        "/proxy/local-conda/channeldata.json",
    ] {
        let req = TestRequest::default()
            .method(actix_web::http::Method::HEAD)
            .uri(path)
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request();
        let resp = call_service(&app, req).await;
        assert!(
            resp.status().is_success(),
            "HEAD {path} must reach the handler — a rejected probe makes conda \
             fall back as if the document did not exist, got {}",
            resp.status()
        );
    }
}
