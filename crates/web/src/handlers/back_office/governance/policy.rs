//! The `policy` table's admin API — RFC 0015 §6.3.
//!
//! §4.1 is the reason this exists rather than a config block: *"Package- and
//! version-level policy cannot live in the config file: a registry with 200 000
//! packages will not enumerate them in TOML, let alone their two million
//! versions."* The registry and namespace tiers are TOML and are reviewed like
//! any other change; these two are written here.
//!
//! # Why the route names the tier
//!
//! `…/policy/package/{package}` and `…/policy/version/{package}/{version}`
//! rather than one route that infers the tier from whether a version was
//! supplied. §4.1 makes the version tier a *fourth level, not a special case*,
//! and a caller who omits a version by mistake would otherwise silently write
//! the package tier — an override one level shallower than intended, applying to
//! every version of the package rather than the one they meant.
//!
//! # Admin-only, for now
//!
//! `require_admin`, matching every other handler in this module. §4.2 defines no
//! verb for writing policy, and inventing one here would put a permission in the
//! model that the RFC has not argued for — `gates:exempt` (§4.5) is the one
//! policy-adjacent verb this document defines, and it governs a *field* rather
//! than this endpoint.

use std::sync::Arc;

use actix_web::{delete, get, put, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{
    entities::{Immutable, QuotaRules, RuleOverride, VersioningRules, Visibility},
    ports::{version_node_key, NodeKind, PolicyRepository, StoredPolicy},
};

use crate::{error::AppError, extractors::AuthIdentity};

/// One node's policy, on the wire.
///
/// Every field is optional and **absent means inherit** — the same distinction
/// the stored row makes, carried onto the API so a client can express it. A
/// client that sent `visibility: "public"` where it meant "leave it alone" would
/// be writing an override with a default value, which stops the tier above from
/// applying.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct PolicyDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerelease_visibility: Option<Visibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versioning: Option<VersioningDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<QuotaDto>,
    /// Gate overrides, one entry per gate. Composes **per gate** (§4.1), so an
    /// entry here replaces that gate's settings and leaves every other gate
    /// running — sending an empty list does not disable anything, it declares no
    /// overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleOverrideDto>,
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct VersioningDto {
    #[serde(default)]
    pub enforce_semver: bool,
    /// RFC 0015 §4.7 — evaluate, record, refuse nothing.
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default = "default_true")]
    pub allow_prerelease: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_pattern: Option<String>,
    #[serde(default)]
    pub immutable: Immutable,
    #[serde(default)]
    pub monotonic: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct QuotaDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes_per_user: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_packages_per_user: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_threshold_pct: Option<u8>,
    #[serde(default)]
    pub block: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RuleOverrideDto {
    /// The gate's name, spelled as `[[registries.rules]]`'s `kind` tag does.
    pub gate: String,
    /// The gate's settings, in the same shape the config block uses.
    pub settings: serde_json::Value,
}

impl PolicyDto {
    fn into_stored(self, registry: &str, node_kind: NodeKind, node_key: String) -> StoredPolicy {
        StoredPolicy {
            registry: registry.to_owned(),
            node_kind,
            node_key,
            visibility: self.visibility,
            prerelease_visibility: self.prerelease_visibility,
            versioning: self.versioning.map(|v| VersioningRules {
                enforce_semver: v.enforce_semver,
                allow_prerelease: v.allow_prerelease,
                version_pattern: v.version_pattern,
                immutable: v.immutable,
                monotonic: v.monotonic,
                dry_run: v.dry_run,
            }),
            quota: self.quota.map(|q| QuotaRules {
                max_bytes_per_user: q.max_bytes_per_user,
                max_packages_per_user: q.max_packages_per_user,
                warn_threshold_pct: q.warn_threshold_pct,
                block: q.block,
            }),
            rules: self
                .rules
                .into_iter()
                .map(|r| RuleOverride {
                    gate: r.gate,
                    settings: r.settings,
                })
                .collect(),
            set_by: None,
        }
    }

    fn from_stored(p: StoredPolicy) -> Self {
        Self {
            visibility: p.visibility,
            prerelease_visibility: p.prerelease_visibility,
            versioning: p.versioning.map(|v| VersioningDto {
                enforce_semver: v.enforce_semver,
                allow_prerelease: v.allow_prerelease,
                version_pattern: v.version_pattern,
                immutable: v.immutable,
                monotonic: v.monotonic,
                dry_run: v.dry_run,
            }),
            quota: p.quota.map(|q| QuotaDto {
                max_bytes_per_user: q.max_bytes_per_user,
                max_packages_per_user: q.max_packages_per_user,
                warn_threshold_pct: q.warn_threshold_pct,
                block: q.block,
            }),
            rules: p
                .rules
                .into_iter()
                .map(|r| RuleOverrideDto {
                    gate: r.gate,
                    settings: r.settings,
                })
                .collect(),
        }
    }
}

/// Reject a coordinate that could become a storage key or a node key it should
/// not.
///
/// The same edge validation every publish handler runs, for the same reason: a
/// `node_key` built from a path segment with `..` in it is a policy written on a
/// coordinate the caller did not name.
fn validate_coordinate(package: &str, version: Option<&str>) -> Result<(), AppError> {
    batlehub_core::services::validate_package_name(package)
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    if let Some(v) = version {
        if v.is_empty() || v.contains("..") || v.contains('/') || v.contains('\\') {
            return Err(AppError::bad_request(
                "version must not be empty or contain path separators",
            ));
        }
    }
    Ok(())
}

// ── package tier ─────────────────────────────────────────────────────────────

/// Read the policy written on a package.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/policy/package/{package}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
    ),
    responses(
        (status = 200, description = "The node's policy; every absent field inherits", body = PolicyDto),
        (status = 403, description = "`owners:read` required"),
        (status = 404, description = "No policy is written on this node"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/policy/package/{package:.*}")]
pub async fn get_package_policy(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersRead,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, None)?;

    match store
        .policy_on_node(&registry, NodeKind::Package, &package)
        .await
        .map_err(AppError::from)?
    {
        // `404` rather than an empty document, and the distinction matters: an
        // empty `PolicyDto` is a legal thing to *send* (it clears the node), so
        // returning one for an absent node would make "nothing is written here"
        // and "everything here is cleared" the same answer.
        None => Err(AppError::not_found(format!(
            "no policy is written on package '{package}' in registry '{registry}'"
        ))),
        Some(p) => Ok(HttpResponse::Ok().json(PolicyDto::from_stored(p))),
    }
}

/// Write the policy on a package, replacing whatever was there.
///
/// Replaces **this node's** declaration; what it inherits is unaffected. An
/// empty body clears the node.
#[utoipa::path(
    put,
    path = "/api/v1/admin/registries/{registry}/policy/package/{package}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
    ),
    request_body = PolicyDto,
    responses(
        (status = 204, description = "Policy written"),
        (status = 400, description = "A field this tier may not carry"),
        (status = 403, description = "`owners:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/registries/{registry}/policy/package/{package:.*}")]
pub async fn put_package_policy(
    path: web::Path<(String, String)>,
    body: web::Json<PolicyDto>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersWrite,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, None)?;

    let mut stored = body
        .into_inner()
        .into_stored(&registry, NodeKind::Package, package);
    stored.set_by = identity.0.user_id.clone();
    store.put_policy(stored).await.map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

/// Clear the policy written on a package.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/policy/package/{package}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
    ),
    responses(
        (status = 204, description = "Policy cleared; absent is not an error"),
        (status = 403, description = "`owners:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/policy/package/{package:.*}")]
pub async fn delete_package_policy(
    path: web::Path<(String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersWrite,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, None)?;
    store
        .delete_policy(&registry, NodeKind::Package, &package)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

// ── version tier ─────────────────────────────────────────────────────────────

/// Read the policy written on one version.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/policy/version/{package}/{version}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
        ("version" = String, Path, description = "Version"),
    ),
    responses(
        (status = 200, description = "The node's policy; every absent field inherits", body = PolicyDto),
        (status = 403, description = "`owners:read` required"),
        (status = 404, description = "No policy is written on this node"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/policy/version/{package}/{version}")]
pub async fn get_version_policy(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersRead,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, Some(&version))?;

    match store
        .policy_on_node(
            &registry,
            NodeKind::Version,
            &version_node_key(&package, &version),
        )
        .await
        .map_err(AppError::from)?
    {
        None => Err(AppError::not_found(format!(
            "no policy is written on '{package}@{version}' in registry '{registry}'"
        ))),
        Some(p) => Ok(HttpResponse::Ok().json(PolicyDto::from_stored(p))),
    }
}

/// Write the policy on one version.
///
/// §4.1 limits what this tier may carry, and the port rejects the rest with a
/// `400`: the naming half of `versioning` governs what a version may be
/// *called*, and here the name already exists; `quota` stops at the package
/// tier, because a per-version quota limits a thing published exactly once.
/// `immutable` is the field the tier exists for — one golden build frozen inside
/// a namespace that otherwise permits replacement.
#[utoipa::path(
    put,
    path = "/api/v1/admin/registries/{registry}/policy/version/{package}/{version}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
        ("version" = String, Path, description = "Version"),
    ),
    request_body = PolicyDto,
    responses(
        (status = 204, description = "Policy written"),
        (status = 400, description = "A field the version tier may not carry"),
        (status = 403, description = "`owners:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/registries/{registry}/policy/version/{package}/{version}")]
pub async fn put_version_policy(
    path: web::Path<(String, String, String)>,
    body: web::Json<PolicyDto>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersWrite,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, Some(&version))?;

    let mut stored = body.into_inner().into_stored(
        &registry,
        NodeKind::Version,
        version_node_key(&package, &version),
    );
    stored.set_by = identity.0.user_id.clone();
    store.put_policy(stored).await.map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

/// Clear the policy written on one version.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/policy/version/{package}/{version}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
        ("version" = String, Path, description = "Version"),
    ),
    responses(
        (status = 204, description = "Policy cleared; absent is not an error"),
        (status = 403, description = "`owners:write` required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/policy/version/{package}/{version}")]
pub async fn delete_version_policy(
    path: web::Path<(String, String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let (registry, package, version) = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersWrite,
        Some(&registry),
        &hot,
    )
    .await?;
    validate_coordinate(&package, Some(&version))?;
    store
        .delete_policy(
            &registry,
            NodeKind::Version,
            &version_node_key(&package, &version),
        )
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

// ── gate exemptions (§4.5) ───────────────────────────────────────────────────
//
// A separate endpoint from `put_version_policy` above, and separately gated.
// Writing one is not an admin operation: §4.5 makes it a **permission**,
// `gates:exempt`, "granted by whoever owns the namespace to whoever they trust
// with it". That is the approval model — not a workflow bolted beside the
// permission system, but a permission — and it is what lets the answer scale
// with the estate. A small team grants it alongside `releases:publish` and moves
// on; a regulated one grants it to a security group and nobody else.
//
// It is deliberately not implied by `releases:*`: a team that may publish to
// `@acme/billing` does not thereby get to decide which CVEs stop mattering
// there.

/// The request body for setting an exemption.
///
/// `exempt_until` and `reason` are **required by the type**, not merely
/// validated — the same discipline §4.7 gives `grants.dry_run`, because the
/// realistic failure is not a wrong assessment but a right assessment nobody
/// revisited.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetExemptionRequest {
    pub exempt_until: chrono::DateTime<chrono::Utc>,
    pub reason: String,
}

/// The exemption as it is read back, carrying the self-approval marker.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExemptionDto {
    pub gate: String,
    pub exempt_until: chrono::DateTime<chrono::Utc>,
    pub reason: String,
    pub granted_by: Option<String>,
    /// Set when the principal that granted the exemption also published the
    /// version. **A marker, not a refusal** — see [`GateExemption`].
    pub self_approved: bool,
}

/// Exempt one version from one gate.
///
/// Only `cve_gate` and `license_gate` may be exempted, and the line is not
/// arbitrary: an exemptible gate reports an **assessable finding**, a
/// non-exemptible one establishes an **invariant**. Any other gate is a `400`.
#[utoipa::path(
    put,
    path = "/api/v1/admin/registries/{registry}/policy/version/{package}/{version}/rules/{gate}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
        ("version" = String, Path, description = "Version"),
        ("gate" = String, Path, description = "cve_gate or license_gate"),
    ),
    request_body = SetExemptionRequest,
    responses(
        (status = 200, description = "The exemption as stored", body = ExemptionDto),
        (status = 400, description = "Not an exemptible gate, or a missing/past exempt_until"),
        (status = 403, description = "gates:exempt required on this coordinate"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/registries/{registry}/policy/version/{package}/{version}/rules/{gate}")]
pub async fn set_gate_exemption(
    path: web::Path<(String, String, String, String)>,
    body: web::Json<SetExemptionRequest>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    local_svc: web::Data<Arc<batlehub_core::services::LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    use batlehub_core::entities::{Action, GateExemption, PackageId};

    let (registry, package, version, gate) = path.into_inner();
    validate_coordinate(&package, Some(&version))?;

    // `gates:exempt`, resolved on the coordinate itself — so a namespace owner
    // can delegate it for their namespace without it reaching the rest of the
    // registry. Not `require_admin`: §4.5 is explicit that this is a permission
    // an operator grants, and an admin-only endpoint would make the grant
    // decorative.
    batlehub_core::services::authz::authorize_grants_public(
        &local_svc.hot,
        &PackageId::new(&registry, &package, &version),
        &identity.0,
        Action::GatesExempt,
    )
    .await
    .map_err(AppError::from)?;

    let body = body.into_inner();
    let mut exemption = GateExemption {
        exempt: true,
        gate: gate.clone(),
        exempt_until: body.exempt_until,
        reason: body.reason,
        granted_by: identity.0.user_id.clone(),
        self_approved: false,
    };
    if let Some(reason) = exemption.validate(chrono::Utc::now()) {
        return Err(AppError::bad_request(reason));
    }

    // §4.5: where `gates:exempt` is held by the same principal that published
    // the version, the exemption is still accepted and is **flagged**. Four-eyes
    // enforced by the tool is friction a small team routes around — most often
    // by granting `gates:exempt` more widely, which is strictly worse than what
    // it was trying to prevent.
    exemption.self_approved = match (
        &identity.0.user_id,
        published_by(&local_svc, &registry, &package, &version).await,
    ) {
        (Some(granter), Some(publisher)) => *granter == publisher,
        _ => false,
    };

    // Read-modify-write the node: an exemption is one gate's override, and
    // replacing the node wholesale would drop an exemption on the *other*
    // exemptible gate — the per-gate composition rule (§4.1) applied to the
    // write path as well as to the read one.
    let node_key = version_node_key(&package, &version);
    let mut policy = store
        .policy_on_node(&registry, NodeKind::Version, &node_key)
        .await
        .map_err(AppError::from)?
        .unwrap_or_else(|| StoredPolicy::new(&registry, NodeKind::Version, &node_key));

    let settings = serde_json::to_value(&exemption)
        .map_err(|e| AppError::bad_request(format!("serialising the exemption: {e}")))?;
    let over = RuleOverride {
        gate: gate.clone(),
        settings,
    };
    match policy.rules.iter_mut().find(|r| r.gate == gate) {
        Some(existing) => *existing = over,
        None => policy.rules.push(over),
    }
    policy.set_by = identity.0.user_id.clone();
    store.put_policy(policy).await.map_err(AppError::from)?;

    Ok(HttpResponse::Ok().json(ExemptionDto {
        gate,
        exempt_until: exemption.exempt_until,
        reason: exemption.reason,
        granted_by: exemption.granted_by,
        self_approved: exemption.self_approved,
    }))
}

/// Who published this version, for the self-approval marker.
///
/// `None` when the version is not locally published or the lookup fails, and the
/// marker is then simply absent — a proxied artifact has no publisher on this
/// instance, so there is nobody for the granter to be the same person as.
async fn published_by(
    local_svc: &Arc<batlehub_core::services::LocalRegistryService>,
    registry: &str,
    package: &str,
    version: &str,
) -> Option<String> {
    local_svc
        .backend
        .get_versions(registry, package)
        .await
        .ok()?
        .into_iter()
        .find(|p| p.version == version)?
        .published_by
}

/// Remove a gate exemption.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/registries/{registry}/policy/version/{package}/{version}/rules/{gate}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("package" = String, Path, description = "Package name"),
        ("version" = String, Path, description = "Version"),
        ("gate" = String, Path, description = "cve_gate or license_gate"),
    ),
    responses(
        (status = 204, description = "Exemption removed; absent is not an error"),
        (status = 403, description = "gates:exempt required on this coordinate"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/registries/{registry}/policy/version/{package}/{version}/rules/{gate}")]
pub async fn delete_gate_exemption(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    local_svc: web::Data<Arc<batlehub_core::services::LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    use batlehub_core::entities::{Action, PackageId};

    let (registry, package, version, gate) = path.into_inner();
    validate_coordinate(&package, Some(&version))?;

    batlehub_core::services::authz::authorize_grants_public(
        &local_svc.hot,
        &PackageId::new(&registry, &package, &version),
        &identity.0,
        Action::GatesExempt,
    )
    .await
    .map_err(AppError::from)?;

    let node_key = version_node_key(&package, &version);
    if let Some(mut policy) = store
        .policy_on_node(&registry, NodeKind::Version, &node_key)
        .await
        .map_err(AppError::from)?
    {
        policy.rules.retain(|r| r.gate != gate);
        // An empty policy is a delete, which the port handles — so removing the
        // last exemption on a node leaves no row behind rather than an override
        // that overrides nothing.
        store.put_policy(policy).await.map_err(AppError::from)?;
    }
    Ok(HttpResponse::NoContent().finish())
}

// ── listing live exemptions (§4.8's Exemptions panel) ────────────────────────

/// One live exemption, with everything the panel filters on.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExemptionListEntry {
    pub package: String,
    pub version: String,
    pub gate: String,
    pub exempt_until: chrono::DateTime<chrono::Utc>,
    pub reason: String,
    pub granted_by: Option<String>,
    /// The principal that granted it also published the version.
    ///
    /// **The filter §4.8 asks the panel for**: *show me every exemption nobody
    /// else looked at.* A marker rather than a refusal, because four-eyes
    /// enforced by the tool is friction a small team routes around — most often
    /// by granting `gates:exempt` more widely, which is strictly worse.
    pub self_approved: bool,
    /// Already past. Listed anyway: an expired exemption silences nothing, and a
    /// page whose subject is *what has been weakened and when it lapses* is
    /// exactly where the difference belongs.
    pub expired: bool,
}

/// Every gate exemption written in a registry.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/exemptions",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 200, description = "Live and expired exemptions, soonest expiry first", body = Vec<ExemptionListEntry>),
        (status = 403, description = "`owners:read` required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/exemptions")]
pub async fn list_exemptions(
    path: web::Path<String>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn PolicyRepository>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    use batlehub_core::entities::GateExemption;

    // Reading the list is admin, writing an entry is `gates:exempt`. Not an
    // inconsistency: this is an inventory of every deliberate weakening in the
    // registry, which is a different thing from the authority to add one.
    let registry = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::OwnersRead,
        Some(&registry),
        &hot,
    )
    .await?;

    let now = chrono::Utc::now();
    let mut out: Vec<ExemptionListEntry> = Vec::new();
    for row in store
        .exemptions_in_registry(&registry)
        .await
        .map_err(AppError::from)?
    {
        // `package@version`, split on the **last** `@`: a package name may
        // contain one (`@acme/billing`), and splitting on the first would report
        // the scope as the package and the rest as a version.
        let (package, version) = row
            .node_key
            .rsplit_once('@')
            .map(|(p, v)| (p.to_owned(), v.to_owned()))
            .unwrap_or_else(|| (row.node_key.clone(), String::new()));

        for rule in &row.rules {
            let Ok(mut ex) = serde_json::from_value::<GateExemption>(rule.settings.clone()) else {
                continue;
            };
            if !ex.exempt {
                continue;
            }
            ex.gate = rule.gate.clone();
            out.push(ExemptionListEntry {
                package: package.clone(),
                version: version.clone(),
                gate: ex.gate,
                exempt_until: ex.exempt_until,
                reason: ex.reason,
                granted_by: ex.granted_by,
                self_approved: ex.self_approved,
                expired: ex.exempt_until <= now,
            });
        }
    }
    // Soonest expiry first: the page's job is to surface what is about to lapse
    // or has, not to enumerate alphabetically.
    out.sort_by_key(|e| e.exempt_until);
    Ok(HttpResponse::Ok().json(out))
}
