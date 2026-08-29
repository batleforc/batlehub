//! `immutable` and `monotonic` on the publish path — RFC 0015 §4.5.
//!
//! Both are new: nothing enforced immutability at any level before phase 4, and
//! `monotonic` had no expression at all. They are tested together because they
//! divide one job between them and neither covers the other's half:
//!
//! - `immutable` stops a coordinate being **replaced**.
//! - `monotonic` stops an *older* number being **republished** after a bad
//!   release, which leaves a resolver picking a version that was never meant to
//!   come back — and which `immutable` cannot see, because the coordinate is
//!   genuinely new.
//!
//! The composition rules themselves are unit-tested in `entities::policy`; what
//! is here is the enforcement, through a real publish request.
//!
//! # Why `immutable` is tested against Maven and `monotonic` against npm
//!
//! Not a preference — the two settings have teeth on different paths, and
//! testing both against one ecosystem would have made half of these tests pass
//! for the wrong reason.
//!
//! **A republish is already refused unconditionally** for every registry whose
//! publish goes through `LocalRegistryBackend::publish`: the backend answers
//! `409 already published` before any policy is consulted. So on npm, an
//! `immutable` test passes whatever the setting says, and passes identically
//! with the feature deleted.
//!
//! **Maven is the exception, and it is the exception §4.5's own example is
//! about.** Its non-POM artifacts (the jar, the sources, the checksums) are
//! stored directly rather than through the three-phase publish, so they call
//! `enforce_publish_policy` and then write to storage — and a re-PUT of the same
//! coordinate overwrites. That is where `immutable` decides something, and it is
//! the SNAPSHOT-versus-release distinction the setting exists for. The
//! path-addressed publishers (deb/rpm) take the same route.
//!
//! `monotonic` is the other way round: it constrains the *version row*, so it is
//! meaningful wherever versions are rows, which is npm and every registry like
//! it.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, TestRequest};
use base64::Engine as _;

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{Immutable, RegistryKind, VersioningRules};

const REG: &str = "local-npm";

fn rules(immutable: Immutable, monotonic: bool) -> VersioningRules {
    VersioningRules {
        enforce_semver: false,
        allow_prerelease: true,
        version_pattern: None,
        immutable,
        monotonic,
        dry_run: false,
    }
}

/// An npm publish payload, in the wire format `npm publish` sends.
fn payload(name: &str, version: &str) -> serde_json::Value {
    let tarball = base64::engine::general_purpose::STANDARD.encode(b"fake-tarball-content");
    serde_json::json!({
        "name": name,
        "versions": {
            version: {
                "name": name,
                "version": version,
                "dist": { "shasum": "abc123", "tarball": format!("http://x/{name}/-/{name}-{version}.tgz") },
            }
        },
        "_attachments": {
            format!("{name}-{version}.tgz"): { "content_type": "application/octet-stream", "data": tarball, "length": 20 }
        }
    })
}

/// An app whose registry tier declares `versioning`.
async fn app_with(
    versioning: VersioningRules,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
    Error = actix_web::Error,
> {
    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    with_policy_tiers(
        &parts,
        REG,
        versioning_tiers(REG, RegistryKind::Npm, versioning),
    )
    .await;
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

async fn publish<S: TestService>(app: &S, name: &str, version: &str) -> u16 {
    let req = TestRequest::put()
        .uri(&format!("/proxy/{REG}/{name}"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_json(payload(name, version))
        .to_request();
    call_service(app, req).await.status().as_u16()
}

// ── immutable, against Maven's jar path ──────────────────────────────────────

const MAVEN: &str = "local-maven";

fn pom(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<project>
  <groupId>com.example</groupId>
  <artifactId>mylib</artifactId>
  <version>{version}</version>
  <packaging>jar</packaging>
</project>"#
    )
}

/// A Maven app whose registry tier declares `versioning`.
async fn maven_app_with(versioning: VersioningRules) -> impl TestService {
    let parts = local_only_app_parts(MAVEN, "maven", RegistryMode::Local, false);
    with_policy_tiers(
        &parts,
        MAVEN,
        versioning_tiers(MAVEN, RegistryKind::Maven, versioning),
    )
    .await;
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

/// PUT the POM, which creates the version row.
async fn put_pom<S: TestService>(app: &S, version: &str) -> u16 {
    let req = TestRequest::put()
        .uri(&format!(
            "/proxy/{MAVEN}/maven2/com/example/mylib/{version}/mylib-{version}.pom"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .insert_header(("Content-Type", "application/xml"))
        .set_payload(pom(version))
        .to_request();
    call_service(app, req).await.status().as_u16()
}

/// PUT the jar, which is the path that overwrites.
async fn put_jar<S: TestService>(app: &S, version: &str, bytes: &'static [u8]) -> u16 {
    let req = TestRequest::put()
        .uri(&format!(
            "/proxy/{MAVEN}/maven2/com/example/mylib/{version}/mylib-{version}.jar"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_payload(bytes)
        .to_request();
    call_service(app, req).await.status().as_u16()
}

/// Today's behaviour, and the default (§10 rule 8).
///
/// `never` is the default *because* nothing enforced immutability before phase
/// 4, so any other value would change the meaning of an existing config. This is
/// the test that would fail if someone made `released` the default for being the
/// better policy — which §4.5 agrees it is, and §10 forbids.
#[actix_web::test]
async fn immutable_never_allows_a_jar_to_be_replaced() {
    let app = maven_app_with(rules(Immutable::Never, false)).await;
    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);
    assert!(
        put_jar(&app, "1.0.0", b"second").await < 300,
        "`never` is today's behaviour on this path and must stay it"
    );
}

/// `always`: no version may ever be replaced.
#[actix_web::test]
async fn immutable_always_refuses_a_replacement() {
    let app = maven_app_with(rules(Immutable::Always, false)).await;
    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);
    assert_eq!(put_jar(&app, "1.0.0", b"second").await, 409);
}

/// …including for an admin.
///
/// **The property `immutable` exists for**, and the reason it is a policy rather
/// than a verb: immutability is a property of the resource, the verb is a
/// property of the subject, and a replace needs both. A namespace can therefore
/// be append-only for *everyone*, which no role-based model can say. Every
/// request in this file already carries the admin token, so this test states the
/// claim rather than adding a new mechanism to it — and it is the one that fails
/// if a future `bypass_roles`-shaped escape is added.
#[actix_web::test]
async fn immutable_always_refuses_an_admin_too() {
    let app = maven_app_with(rules(Immutable::Always, false)).await;
    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);

    let req = TestRequest::put()
        .uri(&format!(
            "/proxy/{MAVEN}/maven2/com/example/mylib/1.0.0/mylib-1.0.0.jar"
        ))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .set_payload(b"second".as_slice())
        .to_request();
    assert_eq!(
        call_service(&app, req).await.status().as_u16(),
        409,
        "an invariant an admin can step over is not an invariant"
    );
}

/// A coordinate that does not exist yet is not being replaced.
#[actix_web::test]
async fn immutable_always_allows_a_new_version() {
    let app = maven_app_with(rules(Immutable::Always, false)).await;
    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);
    assert_eq!(put_pom(&app, "2.0.0").await, 201);
    assert!(put_jar(&app, "2.0.0", b"other").await < 300);
}

/// `released`: a release is frozen, a pre-release churns. The Maven shape, on
/// Maven.
#[actix_web::test]
async fn immutable_released_freezes_releases_and_lets_snapshots_churn() {
    let app = maven_app_with(rules(Immutable::Released, false)).await;

    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);
    assert_eq!(
        put_jar(&app, "1.0.0", b"second").await,
        409,
        "a release is immutable"
    );

    // The case phase 4's converged pre-release definition exists for (§4.5).
    // The rule this replaced did a strict `semver::Version::parse`, which fails
    // on `1.1-SNAPSHOT`'s two-component core, fell through its
    // `unwrap_or(false)` and called a SNAPSHOT a **release** — so under this
    // setting it would have frozen exactly the versions Maven expects to churn.
    assert_eq!(put_pom(&app, "1.1-SNAPSHOT").await, 201);
    assert!(put_jar(&app, "1.1-SNAPSHOT", b"build1").await < 300);
    assert!(
        put_jar(&app, "1.1-SNAPSHOT", b"build2").await < 300,
        "a SNAPSHOT is a pre-release and must stay replaceable"
    );
}

// ── monotonic ────────────────────────────────────────────────────────────────

/// A new version must sort strictly above the newest existing one.
#[actix_web::test]
async fn monotonic_refuses_a_version_that_does_not_sort_above_the_newest() {
    let app = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app, "pkg", "2.0.0").await < 300);
    assert_eq!(
        publish(&app, "pkg", "1.9.9").await,
        409,
        "republishing an older number is what monotonic exists to stop"
    );
    assert!(publish(&app, "pkg", "2.0.1").await < 300);
}

/// The first version of a package has nothing to sort above.
#[actix_web::test]
async fn monotonic_allows_the_first_version() {
    let app = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app, "pkg", "0.0.1").await < 300);
}

/// Pre-releases fall out correctly with no special case, because the ordering is
/// semver: `1.3.0-rc1` sorts above `1.2.0`, and `1.2.0-rc1` after `1.2.0` does
/// not. §4.5 states both; this pins them.
#[actix_web::test]
async fn monotonic_handles_prereleases_by_semver_rather_than_by_special_case() {
    let app = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app, "pkg", "1.2.0").await < 300);
    assert!(
        publish(&app, "pkg", "1.3.0-rc1").await < 300,
        "a pre-release of a later version sorts above"
    );

    let app2 = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app2, "pkg", "1.2.0").await < 300);
    assert_eq!(
        publish(&app2, "pkg", "1.2.0-rc1").await,
        409,
        "a pre-release of the version already published sorts below it"
    );
}

/// The version comparison is a *version* comparison, not a string one.
///
/// `"1.9.0" > "1.10.0"` lexicographically, so a string compare would accept
/// `1.9.0` after `1.10.0` — reopening the hole on exactly the version numbers
/// that reach two digits, which is most real packages eventually.
#[actix_web::test]
async fn monotonic_compares_versions_not_strings() {
    let app = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app, "pkg", "1.10.0").await < 300);
    assert_eq!(
        publish(&app, "pkg", "1.9.0").await,
        409,
        "1.9.0 is older than 1.10.0, whatever a string compare says"
    );
}

/// A yanked version still counts as the newest.
///
/// §4.5 states this as a consequence rather than leaving it to be discovered,
/// and it is the half that makes the setting a *security* control rather than a
/// convenience: without it, yanking `2.0.0` would free `1.9.9` to be re-taken by
/// whoever noticed first.
#[actix_web::test]
async fn a_yanked_version_still_counts_as_the_newest() {
    let app = app_with(rules(Immutable::Never, true)).await;
    assert!(publish(&app, "pkg", "2.0.0").await < 300);

    let yank = TestRequest::delete()
        .uri(&format!("/proxy/{REG}/pkg/-rev/1"))
        .insert_header(("Authorization", bearer(ADMIN_TOKEN)))
        .to_request();
    call_service(&app, yank).await;

    assert_eq!(
        publish(&app, "pkg", "1.9.9").await,
        409,
        "a yanked version must still occupy its place in the order"
    );
}

/// Both together, on the path where both are observable.
#[actix_web::test]
async fn immutable_and_monotonic_compose() {
    let app = maven_app_with(rules(Immutable::Always, true)).await;
    assert_eq!(put_pom(&app, "1.0.0").await, 201);
    assert!(put_jar(&app, "1.0.0", b"first").await < 300);

    assert_eq!(
        put_jar(&app, "1.0.0", b"second").await,
        409,
        "immutable: the same coordinate"
    );
    assert_eq!(
        put_pom(&app, "0.9.0").await,
        409,
        "monotonic: a new coordinate, an older number"
    );
    assert_eq!(put_pom(&app, "1.0.1").await, 201, "and forward is fine");
}

/// A registry that declares no policy behaves exactly as it did before phase 4.
///
/// The asymmetry with grants, asserted at the enforcement point: grants fail
/// closed when nothing matches, because a union of nothing is nothing. These are
/// constraints, so an absent one must constrain nothing — a fixture that wired
/// no policy tiers must not start refusing publishes.
#[actix_web::test]
async fn a_registry_with_no_policy_is_unconstrained() {
    let app = build_local_registry_app(
        local_registry_app_parts(REG, "npm", RegistryMode::Local, None),
        batlehub_web::CargoIndexMap::default(),
        None,
    )
    .await;

    assert!(publish(&app, "pkg", "2.0.0").await < 300);
    assert!(publish(&app, "pkg", "1.0.0").await < 300, "no monotonic");
}

// ── visibility as a publish-time default (§4.5) ──────────────────────────────

/// A namespace's `visibility` is the default applied to what is published into
/// it — "replacing 'public unless someone sets it'".
#[actix_web::test]
async fn a_tier_visibility_becomes_the_published_default() {
    use batlehub_core::entities::{RegistryPolicyTiers, Tier, Visibility};

    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    let mut tiers = RegistryPolicyTiers::open(RegistryKind::Npm, REG);
    tiers.registry.visibility = Some(Visibility::Internal);
    with_policy_tiers(&parts, REG, tiers).await;
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    assert!(publish(&app, "pkg", "1.0.0").await < 300);

    let stored = local_svc
        .backend
        .get_versions(REG, "pkg")
        .await
        .expect("versions");
    assert_eq!(
        stored[0].visibility,
        Visibility::Internal,
        "the tier default must reach the stored row, not just the resolver"
    );
    // Named so the import is used and the tier vocabulary stays visible here.
    let _ = Tier::Namespace;
}

/// `prerelease_visibility` is the same default for a pre-release, which is what
/// `[registries.beta_channel]` becomes (§10 rule 6).
///
/// Both halves asserted in one test on purpose: the interesting property is that
/// they *differ* for the same package, which is the whole reason the second
/// setting exists.
#[actix_web::test]
async fn a_prerelease_takes_the_prerelease_default() {
    use batlehub_core::entities::{RegistryPolicyTiers, Visibility};

    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    let mut tiers = RegistryPolicyTiers::open(RegistryKind::Npm, REG);
    tiers.registry.visibility = Some(Visibility::Public);
    tiers.registry.prerelease_visibility = Some(Visibility::Team);
    with_policy_tiers(&parts, REG, tiers).await;
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    assert!(publish(&app, "pkg", "1.0.0").await < 300);
    assert!(publish(&app, "pkg", "2.0.0-rc1").await < 300);

    let stored = local_svc
        .backend
        .get_versions(REG, "pkg")
        .await
        .expect("versions");
    let visibility_of = |v: &str| {
        stored
            .iter()
            .find(|p| p.version == v)
            .unwrap_or_else(|| panic!("{v} must be published"))
            .visibility
    };
    assert_eq!(visibility_of("1.0.0"), Visibility::Public);
    assert_eq!(
        visibility_of("2.0.0-rc1"),
        Visibility::Team,
        "a pre-release takes the pre-release default"
    );
}

/// A registry with no visibility declared publishes public, exactly as before
/// phase 4.
#[actix_web::test]
async fn no_declared_visibility_still_publishes_public() {
    use batlehub_core::entities::Visibility;

    let parts = local_registry_app_parts(REG, "npm", RegistryMode::Local, None);
    let local_svc = parts.local_svc.clone();
    let app = build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await;

    assert!(publish(&app, "pkg", "1.0.0").await < 300);
    let stored = local_svc.backend.get_versions(REG, "pkg").await.expect("v");
    assert_eq!(stored[0].visibility, Visibility::Public);
}
