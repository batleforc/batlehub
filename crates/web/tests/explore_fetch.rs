//! The **Fetch this version** button (RFC 0007-bis §4.4, §5.3, §7.5).
//!
//! The button's entire security argument is that it *is* the download path.
//! That argument is only true while it stays the download path, which is a
//! property of one call site and therefore easy to erode — somebody optimising a
//! slow fetch by "reusing the warming service" would silently remove every gate,
//! and it would look like reuse rather than like a hole.
//!
//! So two of the tests here are the ones that would fail against a
//! warming-service implementation, and they are the reason this file exists:
//!
//! - **the refusal**, not just the success: a caller whose role cannot download
//!   gets `403` with the rule's own reason. Warming calls `fetch_artifact`
//!   directly and would have succeeded;
//! - **the audit event**, with the caller as the actor. Warming records none, so
//!   this is the assertion that fails the moment the paths are swapped.
//!
//! A test that only checked the happy path would pass against either.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{AccessAction, EventFilter};

const REG: &str = "local-npm";

fn fetch_uri(name: &str, version: &str) -> String {
    format!("/api/v1/explore/packages/{REG}/{name}/{version}/fetch")
}

/// A proxy-mode app with an upstream that answers, so the fetch has something to
/// pull.
async fn app(
    kind: &str,
) -> (
    impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
    std::sync::Arc<dyn batlehub_core::ports::PackageRepository>,
) {
    let parts = local_registry_app_parts(REG, kind, RegistryMode::Proxy, None);
    let repo = std::sync::Arc::clone(&parts.proxy_svc.repo);
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;
    (app, repo)
}

// ── The happy path ───────────────────────────────────────────────────────────

/// It runs the download, and reports what arrived. The size and duration are
/// there so the row can say what the wait bought — the sampled range is 0.57 MB
/// to 41.7 MB (§13.4), wide enough that "done" alone tells a reader nothing.
#[actix_web::test]
async fn a_fetch_pulls_the_version_and_reports_what_arrived() {
    let (app, _) = app("npm").await;

    let resp = call_service(
        &app,
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200, "the fetch should succeed");

    let body: serde_json::Value = read_body_json(resp).await;
    assert_eq!(body["fetched"], true);
    assert_eq!(body["registry"], REG);
    assert_eq!(body["name"], "widget");
    assert_eq!(body["version"], "1.0.0");
    assert!(body["size_bytes"].as_u64().unwrap() > 0, "{body}");
    assert!(body["duration_ms"].is_number(), "{body}");
}

/// The bytes land in storage through the ordinary path, which is what makes the
/// row change from `upstream` to `proxied` — and what makes SBOM and README
/// extraction run without a second mechanism (§4.4).
#[actix_web::test]
async fn the_fetched_bytes_are_actually_held_afterwards() {
    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Proxy, None);
    let storage = std::sync::Arc::clone(&parts.proxy_svc.storage);
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    // The **proxy** key. `artifact_storage_key` is the `local:` one a *published*
    // artifact goes to, and asking it here is a question always answered "no".
    let key = batlehub_core::services::proxy::proxy_artifact_key(
        &batlehub_core::entities::PackageId::new(REG, "widget", "1.0.0"),
    );
    assert!(!storage.exists(&key).await.unwrap(), "nothing held before");

    let resp = call_service(
        &app,
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert!(
        storage.exists(&key).await.unwrap(),
        "the artifact must be held after a fetch"
    );
}

/// A second press is a cache read dressed as a fetch. `409` says which, so the
/// console refreshes the row rather than reporting a download that did not
/// happen.
#[actix_web::test]
async fn fetching_a_version_already_held_is_a_conflict() {
    let (app, _) = app("npm").await;
    let request = || {
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request()
    };

    assert_eq!(call_service(&app, request()).await.status(), 200);
    let second = call_service(&app, request()).await;
    assert_eq!(second.status(), 409);
    let body: serde_json::Value = read_body_json(second).await;
    assert_eq!(body["code"], "fetch.already-held");
}

// ── The two that would pass against a warming-service implementation ─────────

/// **The refusal.** A caller whose role cannot download a version is refused,
/// with the rule's own reason — the same string the download would have given,
/// so the console shows the operator *why* and `/tools/access-check` explains
/// the same verdict.
///
/// `WarmingService` calls `fetch_artifact` directly and would have succeeded
/// here, which is exactly why this test exists (§5.3, §7.5).
#[actix_web::test]
async fn a_caller_the_rules_would_refuse_is_refused_with_the_rules_reason() {
    let (app, _) = app("npm").await;

    // Anonymous has `releases:read` and not `source:read` in the shared test
    // policy, so the rule engine refuses the bytes while allowing metadata —
    // precisely the split the button must honour.
    let resp = call_service(
        &app,
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403, "the rules must refuse this caller");

    let body: serde_json::Value = read_body_json(resp).await;
    assert_eq!(body["code"], "fetch.denied");
    let message = body["message"].as_str().unwrap_or("");
    assert!(
        !message.is_empty(),
        "the refusal must carry a reason: {body}"
    );
}

/// **The audit event**, with the caller as the actor.
///
/// This is the difference from a page view in one line: a page view has no actor
/// because nobody decided anything; a fetch has one, and the audit log names
/// them. Warming records nothing, so this assertion is what fails the moment the
/// two paths are swapped (§7.5).
#[actix_web::test]
async fn the_fetch_is_audited_with_the_caller_as_the_actor() {
    let (app, repo) = app("npm").await;

    let resp = call_service(
        &app,
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);

    let events = repo
        .list_events(EventFilter {
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("events");
    let download = events
        .iter()
        .find(|e| {
            e.action == AccessAction::Download
                && e.package_id
                    .as_ref()
                    .is_some_and(|p| p.name == "widget" && p.version == "1.0.0")
        })
        .unwrap_or_else(|| panic!("a download event must be recorded; got {events:?}"));

    assert_eq!(
        download.user_id.as_deref(),
        Some("user-1"),
        "the audit log must name the caller, not the server"
    );
}

// ── What it will not offer ───────────────────────────────────────────────────

/// "Fetch this version" has no single meaning for a kind whose artifact is a set
/// of files, so the endpoint refuses with the reason rather than fetching
/// something arbitrary. The console renders the same string beside a button it
/// does not draw (§4.4).
#[actix_web::test]
async fn a_kind_with_no_single_artifact_per_version_refuses_with_its_reason() {
    for (kind, expected) in [("maven", "set of files"), ("terraform", "architecture")] {
        let (app, _) = app(kind).await;
        let resp = call_service(
            &app,
            TestRequest::post()
                .uri(&fetch_uri("widget", "1.0.0"))
                .insert_header(("Authorization", bearer(USER_TOKEN)))
                .to_request(),
        )
        .await;
        assert_eq!(resp.status(), 400, "{kind} must refuse");

        let body: serde_json::Value = read_body_json(resp).await;
        assert_eq!(body["code"], "fetch.unsupported", "{kind}");
        let message = body["message"].as_str().unwrap_or("");
        assert!(
            message.contains(expected),
            "{kind}: the refusal must quote the kind's own reason, got {message:?}"
        );
    }
}

/// The operator's switch. It admits nothing, so its only job is to let an
/// operator keep the console strictly read-only — and it has to actually do
/// that.
#[actix_web::test]
async fn console_fetch_off_refuses_before_anything_is_fetched() {
    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Proxy, None);
    let storage = std::sync::Arc::clone(&parts.proxy_svc.storage);
    {
        let mut hot = parts.local_svc.hot.write().await;
        hot.console_fetch.insert(REG.to_owned(), false);
    }
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    let resp = call_service(
        &app,
        TestRequest::post()
            .uri(&fetch_uri("widget", "1.0.0"))
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = read_body_json(resp).await;
    assert_eq!(body["code"], "fetch.unsupported");

    // And nothing was pulled: a switch that refused the response after making
    // the request would not be the read-only posture an operator set it for.
    let key = batlehub_core::services::proxy::proxy_artifact_key(
        &batlehub_core::entities::PackageId::new(REG, "widget", "1.0.0"),
    );
    assert!(!storage.exists(&key).await.unwrap());
}

/// A registry this instance does not have answers `404`, in the vocabulary of
/// registries — not a `400` about an unknown registry *type*, which would be an
/// answer to a question the caller did not ask.
#[actix_web::test]
async fn an_unknown_registry_is_a_404_rather_than_a_complaint_about_its_type() {
    let (app, _) = app("npm").await;
    let resp = call_service(
        &app,
        TestRequest::post()
            .uri("/api/v1/explore/packages/not-a-registry/widget/1.0.0/fetch")
            .insert_header(("Authorization", bearer(USER_TOKEN)))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), 404, "unexpected {}", resp.status());
}
