//! The registry rule chain, evaluated against a coordinate that carries no
//! resolved upstream metadata.
//!
//! # Why this is a free function over `HotConfigLock`
//!
//! The rule chain — RBAC, block list, release-age, licence and signature gates —
//! is configured per registry in `HotConfig::policies` and was, until this
//! module existed, reachable only through [`ProxyService`](super::ProxyService).
//! That put it on the *proxy* side of a fork every download handler makes:
//!
//! ```text
//! local/hybrid hit  → LocalRegistryService  → visibility only
//! proxy fall-through → ProxyService::handle → the whole chain
//! ```
//!
//! Handlers were expected to call `ProxyService::authorize_read` themselves
//! before taking the local branch. That is a convention, and the 2026-08-26
//! security survey found eight handlers that did not follow it — after the same
//! defect had already been found and fixed once on the OpenVSX route. A
//! convention that has to be re-applied for every new registry adapter is not a
//! control.
//!
//! Both services hold the same `Arc<RwLock<HotConfig>>`, so the chain does not
//! need `ProxyService` at all: it needs the policies. Hoisting it here lets
//! `LocalRegistryService` run the chain inside its own read funnels, which is
//! what makes the guarded path the only path rather than the polite one.

use std::sync::Arc;

use crate::entities::{Identity, PackageId, PackageMetadata};
use crate::error::CoreError;
use crate::rules::{evaluate_rules, RuleContext, RuleDecision};
use crate::services::hot_config::{HotConfigLock, RegistryPolicy};

/// The coordinate the authorization entry points judge when no upstream
/// metadata has been resolved for it — a path-addressed file, or a listing that
/// names no single version. Every version-derived field is `None`, which is what
/// confines these calls to identity-keyed rules.
pub fn synthetic_metadata(package_id: &PackageId) -> PackageMetadata {
    PackageMetadata {
        id: package_id.clone(),
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
        cache_control: None,
    }
}

async fn policy_for(hot: &HotConfigLock, registry: &str) -> Option<Arc<RegistryPolicy>> {
    let hot = hot.read().await;
    hot.policies.get(registry).cloned()
}

/// Authorize a read against a registry's policy rules **without** resolving
/// upstream metadata or streaming an artifact.
///
/// The full chain runs. Callers are the paths that serve *bytes* — a local
/// artifact, a path-addressed deb/rpm file — where the proxy fall-through would
/// have run the same rules against the same synthetic coordinate.
/// Returns `AccessDenied` when the policy denies the read.
pub async fn authorize_read(
    hot: &HotConfigLock,
    package_id: &PackageId,
    identity: &Identity,
    resource_type: &str,
) -> Result<(), CoreError> {
    // Minimal metadata: deb/rpm files have no per-version upstream metadata,
    // and the RBAC rule keys only off the identity. (The proxy fall-through
    // evaluates the same rule set against the same synthetic coordinate.)
    authorize_read_against(
        hot,
        &synthetic_metadata(package_id),
        identity,
        resource_type,
    )
    .await
}

/// [`authorize_read`] against metadata the caller already holds.
///
/// **Prefer this wherever the metadata is real.** `synthetic_metadata` reports
/// `published_at`, `is_signed` and `checksum` as `None`, and two rules read
/// "absent" as "refuse": `require_signed_release` with `deny_missing_signature`,
/// and `release_age` with `deny_missing_timestamp`. Handing them a synthetic
/// coordinate for a version this instance has the row for does not gate the
/// download, it refuses it — every artifact in the registry, including the
/// properly signed ones the operator turned the gate on to require.
///
/// The proxy path has always done this: `ProxyService::handle` resolves the
/// upstream metadata first and judges the chain against *that*. This is the
/// local half of the same rule, for the local half of the same coordinate.
pub async fn authorize_read_against(
    hot: &HotConfigLock,
    metadata: &PackageMetadata,
    identity: &Identity,
    resource_type: &str,
) -> Result<(), CoreError> {
    let policy = policy_for(hot, metadata.id.registry.as_str()).await;
    let empty: Vec<Box<dyn crate::rules::Rule>> = vec![];
    let rules = policy
        .as_ref()
        .map(|p| p.rules.as_slice())
        .unwrap_or(empty.as_slice());

    let ctx = RuleContext {
        identity,
        package: metadata,
        resource_type,
        cache_entry: None,
        requested_version: Some(&metadata.id.version),
    };
    match evaluate_rules(rules, &ctx).await {
        RuleDecision::Deny { reason } => Err(CoreError::AccessDenied(reason)),
        _ => Ok(()),
    }
}

/// The rules whose verdict comes from the **version's own metadata** rather than
/// from the coordinate or the caller.
///
/// `release_age_gate` reads `published_at` and `require_signed_release` reads
/// `is_signed`, and both treat absent as *deny* when configured to. That is the
/// right answer for an upstream that did not supply the fact — which is what the
/// flags are named for — and the wrong one for a coordinate this instance simply
/// does not hold a row for.
const METADATA_DERIVED_RULES: &[&str] = &["release_age_gate", "require_signed_release"];

/// The chain for a coordinate this instance holds **no version row for**.
///
/// Everything runs except [`METADATA_DERIVED_RULES`]. `block_list`, `cve_gate`,
/// `license_gate`, `version_gate`, `deny_latest` and `rbac` all answer from the
/// coordinate and the caller — both of which are in hand — so they judge here
/// exactly as they would anywhere else. A blocked version is still refused, and
/// refused with `AccessDenied` rather than becoming a `NotFound` that a Hybrid
/// registry would hand to its upstream.
///
/// What defers is the judgement that needs a version we do not have. A Hybrid
/// registry reaches this for everything it proxies, and judging that against
/// `published_at: None` / `is_signed: None` would refuse every proxied artifact
/// on a registry with either gate configured — instead of falling through to the
/// path that resolves the real metadata and runs the same chain against it.
///
/// **A skip-list, not an allow-list**, and deliberately: a rule added later runs
/// here by default. If it turns out to read metadata too, the symptom is a
/// visible over-refusal on hybrid reads; an allow-list would instead skip it
/// silently, which is how a `block_list` stops blocking.
pub async fn authorize_unheld_read(
    hot: &HotConfigLock,
    package_id: &PackageId,
    identity: &Identity,
    resource_type: &str,
) -> Result<(), CoreError> {
    let Some(policy) = policy_for(hot, package_id.registry.as_str()).await else {
        return Ok(());
    };
    let metadata = synthetic_metadata(package_id);
    // `.resolve(identity)` on every verdict, exactly as `evaluate_rules` does.
    // A gate with a non-empty `bypass_roles` does not answer `Deny`; it answers
    // `RequireRole { minimum }` and leaves the comparison against the caller to
    // `resolve`. Matching on `Deny` alone therefore reads "admins may bypass
    // this" as "nobody is gated by this", and `version_gate`, `deny_latest` and
    // `trusted_publisher` all become no-ops on this path.
    for rule in policy
        .rules
        .iter()
        .filter(|r| !METADATA_DERIVED_RULES.contains(&r.name()))
    {
        let ctx = RuleContext {
            identity,
            package: &metadata,
            resource_type,
            cache_entry: None,
            requested_version: Some(&package_id.version),
        };
        if let RuleDecision::Deny { reason } = rule.evaluate(&ctx).await.resolve(identity) {
            return Err(CoreError::AccessDenied(reason));
        }
    }
    Ok(())
}

/// Authorize a *listing* — a request for a whole package's version document,
/// not for one version of it.
///
/// Only the identity-keyed `rbac` rule runs. Every other rule in the chain
/// judges a **concrete version**, and a listing has none: the coordinate
/// carries the pseudo-version `"latest"` and metadata that is synthetic by
/// construction (`published_at`, `is_signed` and `checksum` are all `None`,
/// because no upstream document has been resolved for a single version).
///
/// Handing that to the full chain does not gate the listing, it blanks it.
/// `LicenseGateRule` with `allow_unknown = false` finds no licence recorded for
/// `"latest"` and denies; `ReleaseAgeGateRule` with `deny_missing_timestamp =
/// true` sees `published_at: None` and denies; `require_signed_release` sees
/// `is_signed: None` and denies; a `version_gate` allowlist matches nothing
/// against the literal `"latest"`. Each of those turns "one version in this
/// package is gated" into "`npm install` of anything from this registry fails",
/// which is the opposite of letting a resolver route *past* a gated version to
/// one it may have.
///
/// The chain is not skipped, only deferred: it still runs in full on the
/// download that follows, against the concrete version and its real metadata.
pub async fn authorize_listing(
    hot: &HotConfigLock,
    package_id: &PackageId,
    identity: &Identity,
    resource_type: &str,
) -> Result<(), CoreError> {
    let Some(policy) = policy_for(hot, package_id.registry.as_str()).await else {
        return Ok(());
    };
    let metadata = synthetic_metadata(package_id);
    // `.resolve(identity)` for the same reason as `authorize_unheld_read`:
    // `rbac` never answers `RequireRole` today, but this filter is the kind that
    // widens, and a dropped `RequireRole` is a silent allow.
    for rule in policy.rules.iter().filter(|r| r.name() == "rbac") {
        let ctx = RuleContext {
            identity,
            package: &metadata,
            resource_type,
            cache_entry: None,
            requested_version: None,
        };
        if let RuleDecision::Deny { reason } = rule.evaluate(&ctx).await.resolve(identity) {
            return Err(CoreError::AccessDenied(reason));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Role;
    use crate::rules::{resource_type, Rule, VersionGateRule};
    use crate::services::hot_config::HotConfig;
    use tokio::sync::RwLock;

    fn hot_with(rules: Vec<Box<dyn Rule>>) -> HotConfigLock {
        let mut hot = HotConfig::default();
        hot.policies.insert(
            "reg".to_owned(),
            Arc::new(RegistryPolicy {
                metadata_ttl: None,
                rules,
                firewall_only: false,
                serve_stale_metadata: false,
                artifact_ttl: None,
            }),
        );
        Arc::new(RwLock::new(hot))
    }

    fn blocking_gate() -> HotConfigLock {
        hot_with(vec![Box::new(VersionGateRule::new(
            &[],
            &["1.2.3".to_owned()],
            vec![Role::Admin],
        ))])
    }

    /// A gate with a non-empty `bypass_roles` answers `RequireRole`, not `Deny`.
    /// Matching on `Deny` alone let the blocked version through to *everyone* —
    /// the rule became a no-op the moment an operator named a bypass role.
    #[tokio::test]
    async fn a_gate_with_bypass_roles_still_refuses_a_caller_who_lacks_them() {
        let pkg = PackageId::new("reg", "pkg", "1.2.3");
        let err = authorize_unheld_read(
            &blocking_gate(),
            &pkg,
            &Identity::anonymous(),
            resource_type::RELEASES_READ,
        )
        .await
        .expect_err("a blocked version must not be readable by an anonymous caller");
        assert!(matches!(err, CoreError::AccessDenied(_)), "{err:?}");
    }

    /// …and the role the operator named does still bypass it.
    #[tokio::test]
    async fn a_caller_holding_a_bypass_role_is_allowed() {
        let pkg = PackageId::new("reg", "pkg", "1.2.3");
        let admin = Identity {
            user_id: Some("root".to_owned()),
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        };
        authorize_unheld_read(&blocking_gate(), &pkg, &admin, resource_type::RELEASES_READ)
            .await
            .expect("a bypass role must still bypass");
    }

    /// The gate is a gate, not a wall: an unblocked coordinate is unaffected.
    #[tokio::test]
    async fn an_ungated_version_is_allowed() {
        let pkg = PackageId::new("reg", "pkg", "1.2.4");
        authorize_unheld_read(
            &blocking_gate(),
            &pkg,
            &Identity::anonymous(),
            resource_type::RELEASES_READ,
        )
        .await
        .expect("only the blocked version is gated");
    }
}
