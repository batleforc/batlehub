//! `explain` agrees with the decision.
//!
//! RFC 0015 §11.6:
//!
//! > `explain` (§4.8) resolves without performing, which makes it a second
//! > implementation of the thing it describes — and a diagnostic that can
//! > disagree with reality is worse than none, because it is trusted.
//! >
//! > So it is tested as an oracle rather than on its own: for every row of the
//! > §11.1 matrix, the `explain` verdict for that subject/action/resource must
//! > equal the verdict the real request received.
//!
//! # What "the same verdict" can and cannot mean here
//!
//! `explain` answers about **grants alone** — it says so in its `not_covered`
//! field — while a real request also meets per-package visibility, the
//! pre-release channel, the artifact gates and the block layers. So the two
//! agree in one direction unconditionally and in the other conditionally:
//!
//! - **`explain` denies ⇒ the request must be refused.** Grants are the first
//!   gate; nothing behind them can grant what they withheld. A disagreement here
//!   would mean a route reachable without a grant, which is the finding class
//!   this whole RFC exists to close.
//! - **`explain` allows ⇒ the request may still be refused**, by a gate
//!   `explain` does not evaluate. That is not a disagreement, and asserting
//!   equality in both directions would make this file fail for the artifact
//!   gates doing their job.
//!
//! The asymmetry is the point rather than a weakening: the direction that is
//! unconditional is the one where a wrong answer is a disclosure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_config::schema::RegistryMode;

/// Percent-encode a query value.
///
/// Hand-rolled rather than a new dependency: the values here are subject
/// spellings and package names, and the characters that actually need it are
/// `&`, `=`, `#`, `+`, `%` and space. `@`, `:`, `*` and `/` are all legal in a
/// query value and are left alone — encoding them would make the URLs in a
/// failure message harder to read than the thing they are testing.
fn enc(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "%26".to_owned(),
            '=' => "%3D".to_owned(),
            '#' => "%23".to_owned(),
            '+' => "%2B".to_owned(),
            '%' => "%25".to_owned(),
            ' ' => "%20".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

/// Ask `explain` about a subject on a registry.
async fn explain<S: TestService>(
    app: &S,
    registry: &str,
    subject: &str,
    action: &str,
    package: Option<&str>,
) -> Value {
    let mut uri = format!(
        "/api/v1/admin/authz/explain?registry={registry}&subject={}&action={}",
        enc(subject),
        enc(action)
    );
    if let Some(p) = package {
        uri.push_str(&format!("&package={}", enc(p)));
    }
    let req = TestRequest::get()
        .uri(&uri)
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    let resp = call_service(app, req).await;
    assert_eq!(resp.status(), 200, "explain should answer");
    read_body_json(resp).await
}

/// The `authz_matrix.rs` fixture: anonymous granted nothing, one published
/// package, so a refusal can only come from authorization.
///
/// The package is seeded because the positive direction needs something to
/// serve — an unseeded coordinate answers `404` for everyone, and a test that
/// accepted that would assert nothing about the allow.
async fn deny_anonymous_app() -> impl TestService {
    use batlehub_core::entities::{Identity, Role};
    use batlehub_core::services::PublishRequest;
    use sha2::{Digest, Sha256};

    let parts = local_only_app_parts_with_policy(
        "reg",
        "npm",
        RegistryMode::Local,
        true,
        rbac_policy_deny_anonymous,
    )
    .await;

    let artifact = bytes::Bytes::from_static(b"explain-oracle-bytes");
    let checksum = hex::encode(Sha256::digest(&artifact));
    parts
        .local_svc
        .publish(PublishRequest {
            registry: "reg".to_owned(),
            name: "pkg".to_owned(),
            version: "9.8.7".to_owned(),
            artifact,
            checksum,
            index_metadata: serde_json::json!({}),
            unlisted: false,
            publisher: Identity {
                user_id: Some("user-1".to_owned()),
                role: Role::User,
                auth_provider: None,
                groups: vec![],
            },
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect("fixture must publish");

    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

/// The unconditional direction: what `explain` denies, the route refuses.
///
/// The fixture is the one `authz_matrix.rs` uses for its whole negative axis —
/// anonymous granted nothing — so a disagreement here would invalidate that
/// suite rather than this one.
#[actix_web::test]
async fn what_explain_denies_the_route_refuses() {
    let app = deny_anonymous_app().await;

    let doc = explain(&app, "reg", "*", "releases:read", Some("pkg")).await;
    assert_eq!(
        doc["decision"], "deny",
        "anonymous holds nothing on this fixture"
    );

    // The same request, for real, anonymously.
    let req = TestRequest::get().uri("/proxy/reg/pkg").to_request();
    let status = call_service(&app, req).await.status();
    assert_ne!(
        status, 200,
        "explain said deny; the route must not answer 200"
    );
}

/// The other direction, as far as it holds: what `explain` allows, the route
/// serves — on a coordinate no artifact gate objects to.
#[actix_web::test]
async fn what_explain_allows_the_route_serves() {
    let app = deny_anonymous_app().await;

    let doc = explain(&app, "reg", "role:user", "releases:read", Some("pkg")).await;
    assert_eq!(doc["decision"], "allow");

    let req = TestRequest::get()
        .uri("/proxy/reg/pkg")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 200);
}

/// `granted_by` names the tier the grant was written at.
///
/// §11.6: *"the provenance is asserted too — `granted_by` must name the tier a
/// test placed the grant at, since 'which line do I edit' is the entire value
/// and it is the part most likely to drift as resolution changes."*
#[actix_web::test]
async fn provenance_names_the_tier_and_the_subject_form() {
    let app = deny_anonymous_app().await;
    let doc = explain(&app, "reg", "role:user", "releases:read", Some("pkg")).await;

    let entry = doc["resolved"]
        .as_array()
        .expect("resolved is a list")
        .iter()
        .find(|v| v["action"] == "releases:read")
        .expect("releases:read is held");

    assert_eq!(
        entry["granted_by"], "registry:reg",
        "the fixture writes its permissions at registry tier"
    );
    assert_eq!(
        entry["subject"], "role:user",
        "and under the role form, not the wildcard"
    );
}

/// The tiers walked include the ones that granted nothing.
///
/// A tier missing from the list reads as "not considered", which is a different
/// diagnosis from "considered and matched nothing" — and telling those apart is
/// what an operator opens this endpoint for.
#[actix_web::test]
async fn tiers_walked_names_every_tier_including_the_deeper_ones() {
    let app = deny_anonymous_app().await;
    let doc = explain(&app, "reg", "role:user", "releases:read", Some("pkg")).await;

    let tiers: Vec<&str> = doc["tiers_walked"]
        .as_array()
        .expect("tiers_walked is a list")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();

    assert!(tiers.contains(&"registry:reg"), "{tiers:?}");
    assert!(
        tiers.contains(&"package:pkg"),
        "the package tier is considered even with no policy table behind it: {tiers:?}"
    );
}

/// The answer says what it did not look at.
///
/// Same discipline as `access-check`'s `covers` (RFC 0004-bis B4): a bare
/// verdict is ambiguous between "nothing denies this" and "nothing I looked at
/// denies this", and this endpoint looks at exactly one gate of several.
#[actix_web::test]
async fn an_allow_states_what_it_did_not_evaluate() {
    let app = deny_anonymous_app().await;
    let doc = explain(&app, "reg", "role:user", "releases:read", Some("pkg")).await;

    let not_covered = doc["not_covered"]
        .as_array()
        .expect("not_covered is a list");
    assert!(
        !not_covered.is_empty(),
        "an allow that claims to have checked everything would be a lie"
    );
    let joined = not_covered
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("visibility"), "{joined}");
    assert!(joined.contains("block"), "{joined}");
}

// ── Input handling ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn an_unknown_verb_is_a_400_not_a_deny() {
    let app = deny_anonymous_app().await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/explain?registry=reg&subject=role:user&action=releases:raed")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    // A typo'd verb answered "deny" would be a confident answer about a
    // permission that does not exist — the failure the closed enum removed at
    // config load, arriving through the diagnostic instead.
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn a_malformed_subject_is_a_400() {
    let app = deny_anonymous_app().await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/explain?registry=reg&subject=nope&action=releases:read")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

/// A `token:` subject is refused rather than answered about.
///
/// No principal is a machine token yet (§4.3), so any identity this endpoint
/// synthesised would match nobody — and "deny" would then be a statement about
/// the synthesis rather than about the grant.
#[actix_web::test]
async fn a_token_subject_is_refused_rather_than_answered() {
    let app = deny_anonymous_app().await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/explain?registry=reg&subject=token:bot&action=releases:read")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn an_unknown_registry_is_a_404() {
    let app = deny_anonymous_app().await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/explain?registry=nope&subject=*&action=releases:read")
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 404);
}

#[actix_web::test]
async fn a_non_admin_is_forbidden() {
    let app = deny_anonymous_app().await;
    let req = TestRequest::get()
        .uri("/api/v1/admin/authz/explain?registry=reg&subject=*&action=releases:read")
        .insert_header(("Authorization", bearer(USER_TOKEN)))
        .to_request();
    assert_eq!(call_service(&app, req).await.status(), 403);
}

// ── The instance tier, and the fixture that could not have caught it ─────────
//
// §4.1 gained a fifth tier above `registry` (§13.12) because about a dozen
// control endpoints name no registry. `explain` kept building its path with
// `RegistryGrants::path_for`, which cannot see that tier — so it answered about a
// hierarchy **missing its top node**, and a subject granted a verb only there
// resolved to `deny` here and `allow` at the server.
//
// §11.6 is blunt about which direction that is: *"explain denies ⇒ the request is
// refused"* is the unconditional one, *"because a wrong answer is a
// disclosure"* — and here it was an operator reading `deny` and believing a
// coordinate was closed while every request to it succeeded. That is §13.7's
// shadow-mode finding arriving a second time, through a tier instead of a mode.
//
// The oracle could not catch it for the same reason it could not catch the
// shadow: **no fixture had one**. This one does.

/// An app whose `releases:read` comes only from the instance tier.
///
/// Registry-tier grants are deliberately irrelevant to the subject under test,
/// so anything that reaches the package can only have come from the tier above.
async fn instance_only_grant_app() -> impl TestService {
    use batlehub_core::entities::{Action, GrantMap, Node, Role, SubjectMatcher, Tier};

    let parts = local_registry_app_parts("reg", "npm", RegistryMode::Local, None);
    {
        let mut hot = parts.proxy_svc.hot.write().await;
        hot.instance = Some(std::sync::Arc::new(Node::new(
            Tier::Instance,
            "instance",
            Some(
                GrantMap::new()
                    .grant(SubjectMatcher::Anyone, [Action::ReleasesRead])
                    // The control verbs the endpoint itself needs, as rule 5
                    // grants them — otherwise this app cannot answer `explain`.
                    .grant(SubjectMatcher::Role(Role::Admin), [Action::AuthzRead]),
            ),
        )));
    }
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

/// `explain` sees the instance tier, so it does not deny what the server allows.
///
/// The regression, stated as the property §11.6 asks for rather than as the
/// mechanism: an anonymous caller holds `releases:read` at the instance tier and
/// nowhere else, and `explain` must say so.
#[actix_web::test]
async fn explain_sees_a_grant_that_only_the_instance_tier_supplies() {
    let app = instance_only_grant_app().await;
    let answer = explain(&app, "reg", "*", "releases:read", Some("pkg")).await;

    assert_eq!(
        answer["decision"], "allow",
        "the instance tier grants this; `explain` resolving without it would \
         contradict the server — got {answer:?}"
    );
    let provenance = answer["resolved"]
        .as_array()
        .expect("resolved is an array")
        .iter()
        .find(|e| e["action"] == "releases:read")
        .expect("the verb that was asked about must appear in the working");
    assert_eq!(
        provenance["granted_by"], "instance",
        "provenance must name the tier an operator would edit — `granted_by` is \
         the whole value of this endpoint (§4.8), and naming the registry for a \
         grant written above it sends them to the wrong file"
    );
}

/// `tiers_walked` names the instance tier.
///
/// §13.5: *"a tier missing from the list reads as **not considered**, which is a
/// different diagnosis from **considered and matched nothing** — and telling
/// those apart is what an operator opens this endpoint for."* That sentence was
/// written about the package and version tiers; it is just as true one node up.
#[actix_web::test]
async fn tiers_walked_names_the_instance_tier_first() {
    let app = instance_only_grant_app().await;
    let answer = explain(&app, "reg", "*", "releases:read", Some("pkg")).await;
    let walked: Vec<String> = answer["tiers_walked"]
        .as_array()
        .expect("tiers_walked is an array")
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_owned())
        .collect();

    assert_eq!(
        walked.first().map(String::as_str),
        Some("instance"),
        "outermost first, and the instance tier is outermost: {walked:?}"
    );
    assert!(walked.iter().any(|t| t == "registry:reg"));
}
