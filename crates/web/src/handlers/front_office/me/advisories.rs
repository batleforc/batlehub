use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use chrono::{Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::{
    entities::{PackageId, Severity},
    services::{AdminService, LocalRegistryService},
};

use super::require_user_id;
use crate::{error::AppError, extractors::AuthIdentity};

/// RFC 0004 R6: bounded on both axes. A count alone degenerates the moment a CI
/// job pulls twenty versions of one package; a window alone is unbounded for a
/// busy user.
const RECENT_WINDOW_DAYS: i64 = 7;
const RECENT_COORDINATE_COUNT: usize = 5;

/// Cap on owned packages joined against the findings table, so a user who owns
/// a thousand packages does not turn one widget into a thousand-row query.
const MAX_OWNED_PACKAGES: usize = 100;

/// Why a coordinate is in the caller's list.
///
/// The two are different relationships and are labelled rather than merged: you
/// are *exposed to* what you pulled, and you can *fix* what you own (RFC 0004
/// R7). The same coordinate can be both, in which case it appears once under
/// `owned` — the actionable relationship wins. A version you pulled and a
/// different version you own are two coordinates and stay two rows (R15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdvisoryRelation {
    /// The caller downloaded this package recently.
    Pulled,
    /// The caller owns this package, directly or through a group.
    Owned,
}

/// One advisory finding, flattened to what a reader needs to act.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyAdvisoryFindingDto {
    /// Identifier in the source database (`RUSTSEC-…`, `GHSA-…`, `CVE-…`).
    pub osv_id: String,
    pub severity: Severity,
    pub summary: String,
    /// First version that fixes it, when the database reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed_version: Option<String>,
}

/// One coordinate with known advisories, and why it concerns the caller.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyAdvisoryDto {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub relation: AdvisoryRelation,
    /// The worst severity among `findings`, so a caller can rank rows without
    /// reducing the list themselves.
    pub highest_severity: Severity,
    pub findings: Vec<MyAdvisoryFindingDto>,
}

/// Known advisories on what the caller recently pulled and on what they own.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyAdvisoriesResponse {
    /// Coordinates with findings, worst severity first. Empty means exactly
    /// that: nothing you pulled recently or own has a known advisory.
    pub advisories: Vec<MyAdvisoryDto>,
    /// How many days back "recently pulled" reaches, so a reader knows what
    /// the empty case is a statement about.
    pub window_days: i64,
    /// True when this instance records no vulnerability findings at all
    /// (no SBOM re-scan configured). An empty list then means "we do not know",
    /// not "you are clear" — a distinction the widget must not blur.
    pub scanning_available: bool,
}

/// Advisories affecting the caller: what they pulled, and what they own.
///
/// The join happens here rather than in the browser on purpose (RFC 0004 §5.3):
/// asking the client to fetch findings per coordinate would be N requests, and
/// it would write the caller's package list into the network log of whatever
/// machine they are sitting at.
///
/// Takes no `user_id`. Both halves are scoped by ports that cannot be called
/// without a subject — `list_own_downloads` and `list_owned_by`.
#[utoipa::path(
    get,
    path = "/api/v1/me/advisories",
    tag = "user",
    responses(
        (status = 200, description = "Advisories on the caller's recent pulls and owned packages", body = MyAdvisoriesResponse),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me/advisories")]
pub async fn my_advisories(
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    let user_id = require_user_id(&identity)?;
    let since = Utc::now() - Duration::days(RECENT_WINDOW_DAYS);

    let recent = admin_svc
        .recent_own_coordinates(user_id, since, RECENT_COORDINATE_COUNT)
        .await
        .map_err(AppError::from)?;

    // Owned packages carry no version — ownership is per package — so each
    // resolves to its latest published version before it can be looked up in a
    // findings table keyed by coordinate.
    let owned = match &local_svc.ownership {
        Some(store) => store
            .list_owned_by(&identity.0)
            .await
            .map_err(AppError::from)?,
        None => vec![],
    };

    let mut coordinates: Vec<(PackageId, AdvisoryRelation)> = Vec::new();

    for (registry, name) in owned.into_iter().take(MAX_OWNED_PACKAGES) {
        let versions = local_svc
            .backend
            .get_versions(&registry, &name)
            .await
            .map_err(AppError::from)?;
        // `get_versions` is newest-last by publish order; the newest is the one
        // an owner would fix.
        if let Some(latest) = versions.last() {
            coordinates.push((
                PackageId::new(&registry, &name, &latest.version),
                AdvisoryRelation::Owned,
            ));
        }
    }

    // Owned wins over pulled for the *same coordinate*: "you can fix this" is
    // the more actionable of the two labels, and one row per coordinate keeps
    // the widget from saying the same thing twice. A different version of the
    // same package is a different coordinate and keeps its own row — you are
    // exposed to the version you pulled whether or not you own a later one
    // (RFC 0004 R15).
    for (pkg, _) in recent {
        let already = coordinates.iter().any(|(c, _)| {
            c.registry == pkg.registry && c.name == pkg.name && c.version == pkg.version
        });
        if !already {
            coordinates.push((pkg, AdvisoryRelation::Pulled));
        }
    }

    let ids: Vec<PackageId> = coordinates.iter().map(|(p, _)| p.clone()).collect();
    let found = admin_svc
        .list_vulnerabilities_for(&ids)
        .await
        .map_err(AppError::from)?;

    let mut advisories: Vec<MyAdvisoryDto> = found
        .into_iter()
        .filter_map(|(pkg, findings)| {
            let relation = coordinates
                .iter()
                .find(|(c, _)| {
                    c.registry == pkg.registry && c.name == pkg.name && c.version == pkg.version
                })
                .map(|(_, r)| *r)?;
            let highest_severity = findings
                .iter()
                .map(|f| f.severity)
                .max()
                .unwrap_or(Severity::Unknown);
            Some(MyAdvisoryDto {
                registry: pkg.registry,
                name: pkg.name,
                version: pkg.version,
                relation,
                highest_severity,
                findings: findings
                    .into_iter()
                    .map(|f| MyAdvisoryFindingDto {
                        osv_id: f.osv_id,
                        severity: f.severity,
                        summary: f.summary,
                        fixed_version: f.fixed_version,
                    })
                    .collect(),
            })
        })
        .collect();

    // Worst first, then a stable key so two calls agree.
    advisories.sort_by(|a, b| {
        b.highest_severity
            .cmp(&a.highest_severity)
            .then_with(|| a.registry.cmp(&b.registry))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(HttpResponse::Ok().json(MyAdvisoriesResponse {
        advisories,
        window_days: RECENT_WINDOW_DAYS,
        scanning_available: admin_svc.vuln_repo.is_some(),
    }))
}
