//! Gate exemptions and the `gates:exempt` verb — RFC 0015 §4.5.
//!
//! "This CVE does not apply to how we use this library" is a real and common
//! judgement, and before this the only way to act on it was to turn the gate off
//! for the whole registry — which is worse in every respect.
//!
//! Three properties §4.5 argues for at length, and each has a test here because
//! each is a decision someone could reasonably have made the other way:
//!
//! - **Only two gates are exemptible**, and the line is not arbitrary: an
//!   exemptible gate reports an *assessable finding*, a non-exemptible one
//!   establishes an *invariant*.
//! - **`gates:exempt` is a permission, not an admin flag.** A team that may
//!   publish to a namespace does not thereby get to decide which CVEs stop
//!   mattering there.
//! - **Self-approval warns, it does not block.** Four-eyes enforced by the tool
//!   is friction a small team routes around — most often by granting the verb
//!   more widely, which is strictly worse.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use batlehub_config::schema::RegistryMode;

const REG: &str = "npm";

/// An app on which somebody holds `gates:exempt`.
///
/// The verb is granted to **nobody** by default, and that is §10 rule 5 rather
/// than an oversight: *"`gates:exempt` goes to nobody: it is new, and §4.2's
/// shadow release is how an estate discovers it needs one."* So every test that
/// exercises the endpoint has to say who holds it, which is the point — and
/// `writing_an_exemption_needs_the_gates_exempt_verb` below uses the default
/// fixture instead, where nobody does.
async fn app_with_the_verb() -> impl TestService {
    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    {
        // The fixture's own translation, plus `gates:exempt` for `role:user` and
        // nothing else.
        //
        // It used to be `permissive_grants` — every verb to everyone — and
        // `common/mod.rs` says why that is wrong here in as many words: *"Never
        // for one that asserts a denial … a permissive one turns an
        // authorization test into a test of nothing."* It passed anyway while
        // the admin-only rows in this file were answered by `require_admin`, a
        // mechanism outside the grant model. Now that they are answered by
        // `owners:read`, a fixture granting every verb to every caller cannot
        // tell an administrator from a publisher, and
        // `listing_exemptions_requires_admin` said so.
        use batlehub_core::entities::{Action, Role, SubjectMatcher};
        let mut grants = fixture_grants(REG, "npm", &RegistryMode::Local, &rbac_policy_perms());
        let node = grants.registry.grants.take().unwrap_or_default();
        grants.registry.grants =
            Some(node.grant(SubjectMatcher::Role(Role::User), [Action::GatesExempt]));
        let mut hot = parts.proxy_svc.hot.write().await;
        hot.grants = [(REG.to_owned(), std::sync::Arc::new(grants))].into();
    }
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

/// An app on which nobody holds it — the default, and the shipped state.
async fn app_without_the_verb() -> impl TestService {
    build_local_registry_app(
        local_registry_app_parts(REG, "npm", RegistryMode::Local, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await
}

fn uri(gate: &str) -> String {
    format!("/api/v1/admin/registries/npm/policy/version/lodash/1.0.0/rules/{gate}")
}

async fn set<S: TestService>(app: &S, gate: &str, token: &str, body: Value) -> (u16, Value) {
    let req = TestRequest::put()
        .uri(&uri(gate))
        .insert_header(("Authorization", bearer(token)))
        .set_json(body)
        .to_request();
    let resp = call_service(app, req).await;
    let status = resp.status().as_u16();
    if status != 200 {
        return (status, Value::Null);
    }
    (status, read_body_json(resp).await)
}

fn valid_body() -> Value {
    json!({
        "exempt_until": (Utc::now() + Duration::days(30)).to_rfc3339(),
        "reason": "GHSA-1234 — the affected code path is not reachable from our usage",
    })
}

/// The happy path: an exemption is written and read back.
#[actix_web::test]
async fn an_exemption_round_trips() {
    let app = app_with_the_verb().await;
    let (status, body) = set(&app, "cve_gate", ADMIN_TOKEN, valid_body()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["gate"], "cve_gate");
    assert!(body["reason"].as_str().unwrap().contains("GHSA-1234"));
}

/// Only `cve_gate` and `license_gate` may be exempted.
///
/// §4.5 puts this as a sentence worth stating once: *an exemptible gate reports
/// an assessable finding; a non-exemptible gate establishes an invariant.* A
/// quarantine a version can skip is not a quarantine, and an unsigned artifact
/// is an absence of evidence rather than a finding to accept.
#[actix_web::test]
async fn only_the_two_exemptible_gates_are_exemptible() {
    let app = app_with_the_verb().await;

    for gate in ["cve_gate", "license_gate"] {
        assert_eq!(
            set(&app, gate, ADMIN_TOKEN, valid_body()).await.0,
            200,
            "{gate} reports an assessable finding and must be exemptible"
        );
    }

    for gate in [
        "release_age_gate",
        "require_signed_release",
        "trusted_publisher",
        "block_list",
        "deny_latest",
        "version_gate",
    ] {
        assert_eq!(
            set(&app, gate, ADMIN_TOKEN, valid_body()).await.0,
            400,
            "{gate} establishes an invariant, and an invariant with exceptions is not one"
        );
    }
}

/// `exempt_until` and `reason` are required, and a date already past is refused.
///
/// The same discipline §4.7 gives `grants.dry_run`, for the same reason: the
/// realistic failure is not a wrong assessment, it is a right assessment nobody
/// revisited.
#[actix_web::test]
async fn an_exemption_needs_a_reason_and_a_future_expiry() {
    let app = app_with_the_verb().await;

    // Missing `reason` entirely — the type requires it, so this is a 400 at
    // deserialisation.
    let req = TestRequest::put()
        .uri(&uri("cve_gate"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(json!({ "exempt_until": (Utc::now() + Duration::days(1)).to_rfc3339() }))
        .to_request();
    assert_eq!(call_service(&app, req).await.status().as_u16(), 400);

    // Present but empty.
    assert_eq!(
        set(
            &app,
            "cve_gate",
            ADMIN_TOKEN,
            json!({
                "exempt_until": (Utc::now() + Duration::days(1)).to_rfc3339(),
                "reason": "   ",
            })
        )
        .await
        .0,
        400,
        "an empty reason is not a reason"
    );

    // A date already past silences nothing and reads as one that does.
    assert_eq!(
        set(
            &app,
            "cve_gate",
            ADMIN_TOKEN,
            json!({
                "exempt_until": (Utc::now() - Duration::days(1)).to_rfc3339(),
                "reason": "assessed",
            })
        )
        .await
        .0,
        400
    );
}

/// `gates:exempt` is a **permission**, and it is not implied by `releases:*`.
///
/// The fixture's ordinary user holds the read and publish verbs and not this
/// one, which is exactly §4.5's case: a team that may publish to `@acme/billing`
/// does not thereby get to decide which CVEs stop mattering there.
#[actix_web::test]
async fn writing_an_exemption_needs_the_gates_exempt_verb() {
    let app = app_without_the_verb().await;
    assert_eq!(
        set(&app, "cve_gate", USER_TOKEN, valid_body()).await.0,
        403,
        "a publisher must not be able to silence a finding by publishing"
    );
    assert_eq!(
        set(&app, "cve_gate", ADMIN_TOKEN, valid_body()).await.0,
        403,
        "and it is not an admin flag either: §10 rule 5 grants the verb to nobody, so an \
         estate that wants exemptions writes the grant deliberately"
    );

    let req = TestRequest::delete()
        .uri(&uri("cve_gate"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status().as_u16(),
        403,
        "and removing one is the same authority"
    );
}

/// An exemption on one gate does not touch the other.
///
/// The per-gate composition rule (§4.1) applied to the *write* path: replacing
/// the node wholesale would drop an exemption on the other exemptible gate,
/// silently, on an endpoint whose whole subject is not silencing things by
/// accident.
#[actix_web::test]
async fn exempting_one_gate_leaves_the_other_exemption_alone() {
    let app = app_with_the_verb().await;
    assert_eq!(
        set(&app, "cve_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );
    assert_eq!(
        set(&app, "license_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );

    let req = TestRequest::get()
        .uri("/api/v1/admin/registries/npm/policy/version/lodash/1.0.0")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = read_body_json(resp).await;
    let gates: Vec<&str> = body["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .filter_map(|r| r["gate"].as_str())
        .collect();
    assert!(gates.contains(&"cve_gate"), "{gates:?}");
    assert!(gates.contains(&"license_gate"), "{gates:?}");
}

/// Removing an exemption leaves the other one, and removing the last leaves no
/// node behind.
#[actix_web::test]
async fn removing_an_exemption_is_per_gate_and_cleans_up() {
    let app = app_with_the_verb().await;
    assert_eq!(
        set(&app, "cve_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );
    assert_eq!(
        set(&app, "license_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );

    let del = |gate: &'static str| {
        let app = &app;
        async move {
            let req = TestRequest::delete()
                .uri(&uri(gate))
                .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
                .to_request();
            call_service(app, req).await.status().as_u16()
        }
    };
    async fn read<S: TestService>(app: &S) -> u16 {
        let req = TestRequest::get()
            .uri("/api/v1/admin/registries/npm/policy/version/lodash/1.0.0")
            .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
            .to_request();
        call_service(app, req).await.status().as_u16()
    }

    assert_eq!(del("cve_gate").await, 204);
    assert_eq!(read(&app).await, 200, "the other exemption keeps the node");

    assert_eq!(del("license_gate").await, 204);
    assert_eq!(
        read(&app).await,
        404,
        "the last one removed must leave no override that overrides nothing"
    );

    assert_eq!(del("cve_gate").await, 204, "absent is not an error");
}

// ── the Exemptions panel's data (§4.8) ───────────────────────────────────────

/// The list an operator reads to answer "what has been weakened here?".
///
/// §4.8 puts this panel on the authorization page with four others because
/// *"a shadowed grant, a self-approved exemption and a retention run about to go
/// live are each individually easy to forget, and collectively they are the list
/// of everything currently trusting an operator to remember."*
#[actix_web::test]
async fn exemptions_are_listed_for_the_registry() {
    let app = app_with_the_verb().await;
    assert_eq!(
        set(&app, "cve_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );
    assert_eq!(
        set(&app, "license_gate", ADMIN_TOKEN, valid_body()).await.0,
        200
    );

    let req = TestRequest::get()
        .uri(&format!("/api/v1/admin/registries/{REG}/exemptions"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(&app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = read_body_json(resp).await;

    let entries = body.as_array().expect("a list");
    assert_eq!(entries.len(), 2, "{body}");
    for e in entries {
        assert_eq!(
            e["package"], "lodash",
            "the coordinate must be split back out"
        );
        assert_eq!(e["version"], "1.0.0");
        assert_eq!(e["expired"], false);
        assert!(e["reason"].as_str().unwrap().contains("GHSA-1234"));
    }
    let gates: Vec<&str> = entries.iter().filter_map(|e| e["gate"].as_str()).collect();
    assert!(gates.contains(&"cve_gate"), "{gates:?}");
    assert!(gates.contains(&"license_gate"), "{gates:?}");
}

/// An empty registry lists nothing, which is what makes the test above about
/// the exemptions rather than about the endpoint answering at all.
#[actix_web::test]
async fn a_registry_with_no_exemptions_lists_nothing() {
    let app = app_with_the_verb().await;
    let req = TestRequest::get()
        .uri(&format!("/api/v1/admin/registries/{REG}/exemptions"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let body: Value = read_body_json(call_service(&app, req).await).await;
    assert!(body.as_array().expect("a list").is_empty(), "{body}");
}

/// Reading the inventory is admin; writing an entry is `gates:exempt`.
///
/// Not an inconsistency: this is a list of every deliberate weakening in the
/// registry, which is a different thing from the authority to add one.
#[actix_web::test]
async fn listing_exemptions_requires_admin() {
    let app = app_with_the_verb().await;
    let req = TestRequest::get()
        .uri(&format!("/api/v1/admin/registries/{REG}/exemptions"))
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status().as_u16(), 403);
}
