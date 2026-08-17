//! Blocking a gem version hides it from the versions API *and* moves the gem
//! document off it.
//!
//! Two documents, read on different paths. `bundle install` resolves a
//! constraint against `/api/v1/versions/{name}.json`; `gem info` and every UI
//! read `/api/v1/gems/{name}.json`, which describes the gem at exactly one
//! version and so has to be rebuilt around a different one rather than filtered.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::call_service;
use serde_json::Value;

async fn app() -> impl TestService {
    proxy_registry_app("local-gems", "rubygems").await
}

fn numbers(doc: &Value) -> Vec<String> {
    doc.as_array()
        .expect("versions is an array")
        .iter()
        .map(|e| e["number"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[actix_web::test]
async fn proxy_versions_api_hides_a_blocked_version() {
    let app = app().await;
    let uri = "/proxy/local-gems/api/v1/versions/rails.json";

    assert_eq!(
        numbers(&get_json(&app, uri).await),
        ["2.0.0-beta.1", "1.1.0", "1.0.0"]
    );

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    assert_eq!(
        numbers(&get_json(&app, uri).await),
        ["2.0.0-beta.1", "1.0.0"],
        "the blocked version is gone and the newest-first order survives"
    );
}

/// The gem document *is* one version. Blocking that version has to move the
/// document to the newest one an operator does allow, or `gem info` keeps
/// naming a release the download gate will refuse.
#[actix_web::test]
async fn proxy_gem_document_moves_off_a_blocked_version() {
    let app = app().await;
    let uri = "/proxy/local-gems/api/v1/gems/rails.json";

    assert_eq!(get_json(&app, uri).await["version"], "1.1.0");

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_json(&app, uri).await;
    assert_eq!(
        after["version"], "1.0.0",
        "1.1.0 is blocked and 2.0.0-beta.1 is a pre-release, so the newest \
         allowed *stable* release wins"
    );
    assert_eq!(
        after["name"], "rails",
        "gem-level fields survive the rebuild"
    );
}

/// The checksum and download URL described the hidden release. Carried onto a
/// different version they are a hash that can never match what is downloaded.
#[actix_web::test]
async fn proxy_gem_document_drops_the_hidden_release_s_own_fields() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_json(&app, "/proxy/local-gems/api/v1/gems/rails.json").await;
    assert!(after.get("sha").is_none(), "stale checksum survived");
    assert!(
        after.get("gem_uri").is_none(),
        "stale download URL survived"
    );
}

#[actix_web::test]
async fn proxy_gem_document_naming_an_allowed_version_is_untouched() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.0.0").await;

    let after = get_json(&app, "/proxy/local-gems/api/v1/gems/rails.json").await;
    assert_eq!(after["version"], "1.1.0");
    assert_eq!(after["sha"], "bbb", "nothing to repair, nothing removed");
}

#[actix_web::test]
async fn proxy_another_gem_is_untouched() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let other = get_json(&app, "/proxy/local-gems/api/v1/versions/sinatra.json").await;
    assert!(numbers(&other).contains(&"1.1.0".to_owned()));
}

/// Hiding governs resolution, not diagnosis.
#[actix_web::test]
async fn proxy_direct_request_for_a_blocked_version_is_still_denied() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let resp = call_service(&app, admin_get("/proxy/local-gems/gems/rails-1.1.0.gem")).await;
    assert_eq!(resp.status(), 403);
}

// ── The compact index (RFC 0009 §7.3) ─────────────────────────────────────────
//
// The tests above cover the JSON APIs, which were filtered all along. They are
// also not what Bundler reads. Bundler resolves from the compact index first,
// and until this phase we served none of it — so `bundle install` fell back to
// `specs.4.8.gz`, the one index `listing_filter()` marks `Unsupported`, and a
// blocked gem version was offered to the resolver, chosen, written to
// `Gemfile.lock`, and only then refused at download.
//
// These assert the leak is closed on the documents the default client uses.

#[actix_web::test]
async fn compact_info_hides_a_blocked_version_from_bundler() {
    let app = app().await;
    let uri = "/proxy/local-gems/info/rails";

    let before = get_text(&app, uri).await;
    assert!(
        before.contains("1.1.0"),
        "the fixture serves it to begin with"
    );

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_text(&app, uri).await;
    assert!(
        !after.contains("1.1.0"),
        "the blocked version is still offered to the resolver:\n{after}"
    );
    assert!(after.contains("1.0.0"), "the allowed versions survive");
    assert!(after.contains("2.0.0-beta.1"));
    assert!(after.starts_with("---\n"), "the separator must survive");
}

/// `/versions` is whole-registry, so its blocked set comes from the 30-second
/// snapshot rather than a per-request query — the same trade conda's
/// `repodata.json` makes, for the same reason. So this must block **before**
/// reading, or it warms the snapshot first and measures the lag instead of the
/// filter. `a_block_does_not_reach_an_already_warm_snapshot` pins that lag.
#[actix_web::test]
async fn compact_versions_hides_a_blocked_version_from_the_registry_index() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_text(&app, "/proxy/local-gems/versions").await;
    let rails = after
        .lines()
        .find(|l| l.starts_with("rails "))
        .expect("rails still listed");
    assert!(
        !rails.contains("1.1.0"),
        "the blocked version survived in /versions: {rails}"
    );
    assert!(rails.contains("1.0.0") && rails.contains("2.0.0-beta.1"));
    assert!(
        after.contains("rack 1.0.0"),
        "an unrelated gem's line is untouched"
    );
    assert!(
        after.starts_with("created_at: "),
        "the header must survive so the document stays parseable"
    );
}

/// The other half of that trade, stated rather than left implicit: a client
/// that read `/versions` before the block keeps seeing the unfiltered index
/// until the snapshot expires.
///
/// The download gate does **not** lag — a direct request for the blocked
/// version is refused immediately either way, which is what keeps the window
/// from being a hole.
#[actix_web::test]
async fn a_block_does_not_reach_an_already_warm_snapshot() {
    let app = app().await;

    // Warm it.
    let before = get_text(&app, "/proxy/local-gems/versions").await;
    assert!(before.contains("rails 1.0.0,1.1.0,2.0.0-beta.1"));

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let after = get_text(&app, "/proxy/local-gems/versions").await;
    assert!(
        after.contains("1.1.0"),
        "the snapshot is still warm, so the listing lags by up to its TTL"
    );

    // ...and the gate that actually refuses the bytes does not lag.
    let denied = admin_get("/proxy/local-gems/gems/rails-1.1.0.gem");
    assert_eq!(
        call_service(&app, denied).await.status(),
        403,
        "the download gate is the half that is never late"
    );
}

/// The checksum keys Bundler's cached copy of `/info/{gem}`. If it did not move
/// when the version list did, a client could keep serving an `/info` fetched
/// before the block — so the block would never reach the resolver.
///
/// Two apps rather than one, because reading the baseline from the app under
/// test would warm its snapshot and the block would not land (see above).
#[actix_web::test]
async fn a_block_moves_the_info_checksum_so_bundler_refetches() {
    fn rails_checksum(doc: &str) -> String {
        doc.lines()
            .find(|l| l.starts_with("rails "))
            .and_then(|l| l.split(' ').nth(2))
            .expect("rails line with a checksum")
            .to_owned()
    }

    let unblocked = app().await;
    let before = rails_checksum(&get_text(&unblocked, "/proxy/local-gems/versions").await);

    let blocked = app().await;
    block_version(&blocked, "local-gems", "rails", "1.1.0").await;
    let after_doc = get_text(&blocked, "/proxy/local-gems/versions").await;

    assert_ne!(
        before,
        rails_checksum(&after_doc),
        "a cached /info would still be served, and the block would not reach the resolver"
    );
    // The untouched gem must NOT churn, or every block change re-downloads
    // every gem's info document.
    assert!(after_doc.contains("rack 1.0.0 99887766554433221100ffeeddccbbaa"));
}

/// `/names` lists gem names and no versions, so a block has nothing to hide in
/// it. Removing the name would tell Bundler the gem does not exist, which is a
/// different and worse answer than "some of its versions are restricted".
#[actix_web::test]
async fn a_block_does_not_remove_the_gem_from_names() {
    let app = app().await;

    block_version(&app, "local-gems", "rails", "1.1.0").await;

    let names = get_text(&app, "/proxy/local-gems/names").await;
    assert!(
        names.contains("rails"),
        "the gem still exists; only one of its versions is restricted"
    );
}
