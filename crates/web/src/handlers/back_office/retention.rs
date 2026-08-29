//! Reclaiming the bytes of locally published versions nobody is using
//! (RFC 0016 §4.2, §4.3), and pinning the ones that must survive it (§4.1).
//!
//! Tombstones and their compaction are next door in [`super::tombstones`]: they
//! are retention of a *different object* — the record a deletion leaves — and
//! share only the config block.
//!
//! The one thing to know before reading further: **retention destroys the only
//! copy.** Cache eviction discards something a re-fetch brings back; a locally
//! published artifact may exist nowhere else on earth. Every default here is set
//! by that asymmetry, `dry_run` is on unless an operator turns it off, and the
//! run refuses rather than guesses whenever it cannot read the signal its policy
//! depends on.

use std::sync::Arc;

use actix_web::{post, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::services::{
    AdminService, LocalRegistryService, RetentionReport, RetentionService,
};

use crate::{error::AppError, extractors::AuthIdentity, handlers::schemas::OkResponse};

#[derive(Debug, Deserialize, IntoParams)]
pub struct RetentionQuery {
    /// Report without reclaiming. Absent falls back to the registry's configured
    /// `dry_run`, which itself defaults to `true`.
    ///
    /// A caller may pass `true` to preview against a registry configured live,
    /// but passing `false` does **not** override a configured `dry_run = true`:
    /// an operator who armed the safety catch should not have it taken off by a
    /// query string.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RetentionDecisionDto {
    pub name: String,
    pub version: String,
    /// Why the version survived, or absent when it is to be reclaimed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_because: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RetentionResponse {
    pub registry: String,
    /// Versions examined.
    pub examined: u64,
    /// Versions reclaimed, or that would be under `dry_run`.
    pub reclaimed: u64,
    /// Versions kept.
    pub kept: u64,
    /// True when nothing was written.
    pub dry_run: bool,
    /// The affected coordinates, `"{name}@{version}"`. The list an operator
    /// actually reads before turning `dry_run` off.
    pub reclaimed_coordinates: Vec<String>,
    /// Per-version decisions, including the kept ones and why.
    pub decisions: Vec<RetentionDecisionDto>,
    /// How many decisions were dropped from `decisions`. Non-zero means the list
    /// is a sample, not the answer.
    pub decisions_truncated: u64,
    /// Set when the run stopped early. `reclaimed_coordinates` is then what was
    /// actually reclaimed before the fault, and everything after it was not
    /// attempted — a partial run that says so, rather than an error that throws
    /// away the record of what already happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_because: Option<String>,
}

impl RetentionResponse {
    fn new(registry: String, report: RetentionReport) -> Self {
        Self {
            registry,
            examined: report.examined,
            reclaimed: report.reclaimed,
            kept: report.kept,
            dry_run: report.dry_run,
            reclaimed_coordinates: report.reclaimed_coordinates,
            decisions: report
                .decisions
                .into_iter()
                .map(|d| RetentionDecisionDto {
                    name: d.name,
                    version: d.version,
                    kept_because: d.kept_because.map(|r| r.as_str().to_owned()),
                })
                .collect(),
            decisions_truncated: report.decisions_truncated,
            incomplete_because: report.incomplete_because,
        }
    }
}

/// Run retention over a local/hybrid registry's published versions (admin).
///
/// **A version survives if *any* configured keep condition matches.** There is
/// no expression to write and no ordering to get wrong: the only way to reclaim
/// a version is for every configured condition to decline to keep it. Wrong
/// configuration therefore fails toward keeping.
///
/// A reclaimed version leaves a tombstone and its number can never be taken
/// again. Freeing disk must not free the *namespace*, or retention becomes a
/// supply-chain mechanism by accident (RFC 0016 §4.2).
///
/// `409` when the registry has no `[registries.retention]` block, or one with no
/// keep condition. Not a silent no-op: an operator calling this believes they
/// are reclaiming space, and a `200 {"reclaimed": 0}` would confirm it.
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/retention",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        RetentionQuery,
    ),
    responses(
        (status = 200, description = "What was reclaimed, or would be under dry_run",
            body = RetentionResponse),
        (status = 403, description = "`retention:run` required"),
        (status = 409, description = "No retention keep condition configured for this registry"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/retention")]
pub async fn run_retention(
    path: web::Path<String>,
    query: web::Query<RetentionQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    admin_svc: web::Data<Arc<AdminService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::RetentionRun,
        Some(&registry),
        &hot,
    )
    .await?;

    // Through the service's own lock rather than a separate extractor: it is the
    // same `Arc`, and taking it from the service means the policy this handler
    // reads cannot be from a different reload than the run underneath it.
    let policy = {
        let snapshot = local_svc.hot.read().await;
        snapshot.retention.get(&registry).cloned()
    };
    let Some(policy) = policy else {
        return Err(AppError::conflict(format!(
            "registry '{registry}' has no [registries.retention] block, so it keeps every \
             published version forever — which is the default. Nothing to reclaim."
        )));
    };
    let mut run_policy = policy.for_run();
    if !run_policy.reclaims_anything() {
        return Err(AppError::conflict(format!(
            "registry '{registry}' has a [registries.retention] block with no keep condition \
             (keep_versions, keep_for_days, keep_if_pulled_days), so retention has no policy to \
             apply. Nothing to reclaim."
        )));
    }

    // `||` rather than `unwrap_or`: the query may only ever make the run *more*
    // conservative. A configured dry_run = true is an operator's decision that a
    // request must not be able to reverse.
    run_policy.dry_run = run_policy.dry_run || query.dry_run.unwrap_or(false);

    // The download signal comes from `AdminService`'s store rather than its own
    // app-data entry: it is the same `PackageRepository` the audit trail is
    // written to, and a second handle on it could be a different one.
    let svc = RetentionService::new(local_svc.get_ref().clone(), Some(admin_svc.repo.clone()));
    let report = svc
        .run(&registry, &run_policy, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(RetentionResponse::new(registry, report)))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RetentionPinRequest {
    pub name: String,
    pub version: String,
    /// `true` pins, `false` unpins.
    pub keep: bool,
}

/// Pin a version against retention, or release the pin (admin).
///
/// A pinned version is never reclaimed by a retention run, whatever the
/// registry's policy says — the escape every automatic policy needs, for the
/// release an LTS customer runs and the pull statistics will eventually stop
/// defending (RFC 0016 §4.1).
///
/// It is a **keep, never a reclaim**. There is deliberately no spelling of this
/// endpoint that makes retention *more* aggressive for one version, because a
/// policy that deletes should not be reachable one version at a time.
///
/// Pinning changes nothing else: the version resolves, downloads and lists
/// exactly as it did. Pinning a coordinate that is already deleted is a no-op —
/// a spent coordinate cannot be protected from a reclamation that can never
/// reach it.
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/retention-pin",
    tag = "back-office",
    params(("registry" = String, Path, description = "Registry name")),
    request_body = RetentionPinRequest,
    responses(
        (status = 200, description = "Pin set or released", body = OkResponse),
        (status = 403, description = "`retention:run` required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/retention-pin")]
pub async fn set_retention_pin(
    path: web::Path<String>,
    body: web::Json<RetentionPinRequest>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::RetentionRun,
        Some(&registry),
        &hot,
    )
    .await?;
    let body = body.into_inner();
    local_svc
        .set_retention_pin(&registry, &body.name, &body.version, body.keep, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(OkResponse::new()))
}
