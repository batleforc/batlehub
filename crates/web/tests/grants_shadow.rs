//! Shadow mode — RFC 0015 §4.7.
//!
//! `grants.dry_run` is *"the most useful setting in this document and the most
//! dangerous"*. It is what makes §10's migration survivable in practice, and it
//! is also, if forgotten, an authorization bypass configured on purpose.
//!
//! Which is why this file leads with the control. Every test here asserts what a
//! shadow **serves**, so each one is only meaningful beside proof that the same
//! request is refused without it — a suite that only checked the permissive half
//! would pass identically against a build with no grants at all.
//!
//! The three properties that matter:
//!
//! - a shadowed node **serves** what its grants would refuse, and **records** it;
//! - an **expired** shadow enforces, because the alternative is a node quietly
//!   serving what it should refuse since a date passed and nobody noticed;
//! - a shadow anywhere on the path covers the coordinate, because a denial is
//!   the *absence* of a grant rather than one node's decision — there is no
//!   originating node to take the shadow from.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, read_body_json, TestRequest};
use chrono::{Duration, Utc};
use serde_json::Value;

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{DryRun, RegistryKind};

const REG: &str = "local-npm";

/// A registry whose grants refuse an ordinary user, optionally in shadow.
///
/// `RegistryGrants::empty()` is the fixture: it grants nobody anything, so
/// every request is a denial and the only variable is whether the shadow serves
/// it.
async fn app_with_shadow(shadow: Option<DryRun>) -> impl TestService {
    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    {
        let mut hot = parts.proxy_svc.hot.write().await;
        let mut grants = batlehub_core::entities::RegistryGrants::empty(REG, RegistryKind::Npm);
        grants.registry = grants.registry.shadowed(shadow);
        hot.grants = [(REG.to_owned(), Arc::new(grants))].into();
        hot.shadow_log = Some(Arc::new(batlehub_core::services::shadow::ShadowLog::new()));
    }
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

async fn read<S: TestService>(app: &S) -> u16 {
    let req = TestRequest::get()
        .uri(&format!("/proxy/{REG}/pkg"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    call_service(app, req).await.status().as_u16()
}

async fn shadow_report<S: TestService>(app: &S) -> Value {
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/shadow")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    read_body_json(resp).await
}

/// **The control.** Without a shadow, the closed registry refuses.
///
/// First in the file on purpose: every other assertion here is about a request
/// being *served*, and none of them means anything unless this one denies.
#[actix_web::test]
async fn without_a_shadow_a_closed_registry_refuses() {
    let app = app_with_shadow(None).await;
    assert_eq!(read(&app).await, 403);
}

/// A shadowed node serves what its grants would refuse.
#[actix_web::test]
async fn an_active_shadow_serves_what_the_grants_would_refuse() {
    let app = app_with_shadow(Some(DryRun {
        until: (Utc::now() + Duration::days(30)).date_naive(),
    }))
    .await;
    assert_ne!(
        read(&app).await,
        403,
        "the shadow must serve the request the control above refuses"
    );
}

/// An **expired** shadow enforces.
///
/// The fail-closed direction, and the only defensible one: the alternative is a
/// node quietly serving what it should refuse because a date passed and nobody
/// noticed — precisely the failure the required `until` exists to prevent.
///
/// Config load refuses to *start* with a past date, so reaching this state means
/// the server has been up since before the expiry. That is the normal way a
/// shadow ends, not an edge case.
#[actix_web::test]
async fn an_expired_shadow_enforces() {
    let app = app_with_shadow(Some(DryRun {
        until: (Utc::now() - Duration::days(1)).date_naive(),
    }))
    .await;
    assert_eq!(
        read(&app).await,
        403,
        "a shadow that has lapsed must start refusing, not keep serving"
    );
}

/// A shadow expiring **today** is still in force.
///
/// `until` is inclusive, which is the reading an operator writing
/// `until = "2026-12-01"` has: the shadow covers that day and enforcement starts
/// the next. The off-by-one in the other direction would end a migration window
/// a day early, silently.
#[actix_web::test]
async fn a_shadow_expiring_today_is_still_active() {
    let app = app_with_shadow(Some(DryRun {
        until: Utc::now().date_naive(),
    }))
    .await;
    assert_ne!(read(&app).await, 403);
}

/// The would-have-been is recorded, with everything needed to act on it.
///
/// §4.7 asks for the record because a shadow with nothing to read is only the
/// dangerous half. The subject is in **grant spelling** so an operator can paste
/// it into the block that would fix this — a bare user id would leave them
/// guessing which of the five subject forms to write.
#[actix_web::test]
async fn a_shadowed_denial_is_recorded_with_its_node_and_subject() {
    let until = (Utc::now() + Duration::days(7)).date_naive();
    let app = app_with_shadow(Some(DryRun { until })).await;

    assert_ne!(read(&app).await, 403);

    let report = shadow_report(&app).await;
    let recent = report["recent"].as_array().expect("recent");
    assert!(!recent.is_empty(), "the would-have-been must be recorded");

    let entry = &recent[0];
    assert_eq!(entry["registry"], REG);
    assert_eq!(entry["node"], format!("registry:{REG}"));
    assert_eq!(entry["shadow_until"], until.to_string());
    assert!(
        entry["subject"].as_str().unwrap().starts_with("user:")
            || entry["subject"].as_str().unwrap().starts_with("role:"),
        "the subject must be in grant spelling: {entry}"
    );
    assert!(
        entry["action"].as_str().unwrap().starts_with("releases:"),
        "and it must name the verb that was missing: {entry}"
    );
}

/// The summary is per node, because that is the shape the question has.
#[actix_web::test]
async fn the_report_summarises_by_node() {
    let app = app_with_shadow(Some(DryRun {
        until: (Utc::now() + Duration::days(7)).date_naive(),
    }))
    .await;
    for _ in 0..3 {
        assert_ne!(read(&app).await, 403);
    }

    let report = shadow_report(&app).await;
    let by_node = report["by_node"].as_array().expect("by_node");
    assert_eq!(by_node.len(), 1, "one node was shadowed: {by_node:?}");
    assert!(
        by_node[0]["count"].as_u64().unwrap() >= 3,
        "every served request counts: {by_node:?}"
    );
}

/// An empty report distinguishes "the shadow is quiet" from "there is no
/// shadow".
///
/// The two look identical in an empty list and mean opposite things: the first
/// says enforcing is safe, the second says nothing was measured. An operator
/// about to flip a migration to enforcing reads this field before the list.
#[actix_web::test]
async fn the_report_says_whether_anything_is_shadowed_at_all() {
    let unshadowed = app_with_shadow(None).await;
    let report = shadow_report(&unshadowed).await;
    assert_eq!(report["no_shadow_configured"], true);
    assert!(report["recent"].as_array().unwrap().is_empty());

    let shadowed = app_with_shadow(Some(DryRun {
        until: (Utc::now() + Duration::days(7)).date_naive(),
    }))
    .await;
    let report = shadow_report(&shadowed).await;
    assert_eq!(
        report["no_shadow_configured"], false,
        "a configured shadow that has served nothing yet is not the same as no shadow"
    );
}

/// The report is admin-only. It names coordinates, subjects and the verbs they
/// lack, which is a map of what is currently reachable and by whom.
#[actix_web::test]
async fn the_report_requires_admin() {
    let app = app_with_shadow(None).await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/shadow")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status().as_u16(), 403);
}

// ── `explain` must not disagree with reality (§4.8, §11.6) ───────────────────

/// A `deny` under an active shadow says so.
///
/// This is the failure §11.6 names: *"a diagnostic that can disagree with
/// reality is worse than none, because it is trusted."* Under a shadow the
/// grants refuse and the server serves, so an operator reading a bare `deny`
/// would conclude the coordinate is closed while every request to it succeeds —
/// the exact misreading a shadow makes possible and this endpoint exists to
/// prevent.
///
/// Reported as a field rather than folded into `decision`, because both facts
/// are true and the operator needs both: the grants refuse, *and* it is being
/// served anyway until the date named.
#[actix_web::test]
async fn explain_discloses_that_a_denial_is_being_shadowed() {
    let until = (Utc::now() + Duration::days(7)).date_naive();
    let app = app_with_shadow(Some(DryRun { until })).await;

    let req = TestRequest::get()
        .uri(&format!(
            "/api/v1/admin/authz/explain?registry={REG}&subject=role%3Auser&action=releases%3Aread"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = read_body_json(resp).await;

    assert_eq!(body["decision"], "deny", "the grants do refuse");
    let note = &body["shadowed_by"];
    assert!(
        !note.is_null(),
        "…and the answer must say the request is served anyway: {body}"
    );
    assert_eq!(note["node"], format!("registry:{REG}"));
    assert_eq!(note["until"], until.to_string());
}

/// Without a shadow the field is absent, so its presence always means something.
#[actix_web::test]
async fn explain_omits_the_shadow_note_when_there_is_none() {
    let app = app_with_shadow(None).await;

    let req = TestRequest::get()
        .uri(&format!(
            "/api/v1/admin/authz/explain?registry={REG}&subject=role%3Auser&action=releases%3Aread"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;

    assert_eq!(body["decision"], "deny");
    assert!(body.get("shadowed_by").is_none(), "{body}");
}

/// An **expired** shadow is not disclosed, because it is not serving anything.
///
/// The field tracks what is happening rather than what is configured — an
/// operator reading it is asking "is this coordinate actually reachable?", and a
/// lapsed shadow answers no.
#[actix_web::test]
async fn explain_omits_the_note_for_an_expired_shadow() {
    let app = app_with_shadow(Some(DryRun {
        until: (Utc::now() - Duration::days(1)).date_naive(),
    }))
    .await;

    let req = TestRequest::get()
        .uri(&format!(
            "/api/v1/admin/authz/explain?registry={REG}&subject=role%3Auser&action=releases%3Aread"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert!(body.get("shadowed_by").is_none(), "{body}");
}

/// The resource attributes §4.8 shows beside the resolved verbs.
///
/// A resolved set that showed `releases:read` without saying the package is
/// `team`-visible answers half the question and reads as the whole one: §4.5's
/// two directions are an **AND**, and a caller needs a grant *and* membership of
/// the audience.
#[actix_web::test]
async fn explain_reports_the_resolved_attributes() {
    use batlehub_core::entities::{Immutable, RegistryPolicyTiers, Visibility};

    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    let mut tiers = RegistryPolicyTiers::open(RegistryKind::Npm, REG);
    tiers.registry.visibility = Some(Visibility::Team);
    tiers.registry.versioning = Some(batlehub_core::entities::VersioningRules {
        enforce_semver: false,
        allow_prerelease: true,
        version_pattern: None,
        immutable: Immutable::Released,
        monotonic: true,
        dry_run: false,
    });
    with_policy_tiers(&parts, REG, tiers).await;
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    let req = TestRequest::get()
        .uri(&format!(
            "/api/v1/admin/authz/explain?registry={REG}&subject=role%3Auser&action=releases%3Aread"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;

    let attrs = &body["attributes"];
    assert_eq!(attrs["visibility"], "team");
    assert_eq!(
        attrs["prerelease_visibility"], "team",
        "a pre-release is not a wider audience by default"
    );
    assert_eq!(attrs["immutable"], "released");
    assert_eq!(attrs["monotonic"], true);
}
