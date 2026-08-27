//! Reading and compacting the record of what was deleted (RFC 0016 §4.4, §4.5).
//!
//! A tombstoned version is absent from every registry-protocol listing and from
//! every resolver's view — it is not installable, and a listing that still named
//! it would produce a build that fails at download. These two endpoints are the
//! exception the RFC names: the audit and ownership views, which are entitled to
//! see that a coordinate existed and is spent.
//!
//! Deleting a version is not here. It belongs to the bulk handler in
//! [`super::bulk`], with the yank and unyank it is a sibling of.

use std::sync::Arc;

use actix_web::{get, post, web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{
    entities::{CompactionReport, Tombstone},
    services::LocalRegistryService,
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

#[derive(Debug, Deserialize, IntoParams)]
pub struct TombstoneQuery {
    /// Narrow the list to one package name. Absent lists every tombstone in the
    /// registry.
    #[serde(default)]
    pub name: Option<String>,
}

/// One deleted coordinate.
///
/// Mirrors [`Tombstone`] rather than re-exporting it, so the API shape is a
/// decision this crate makes: the entity is what the store holds, and the two
/// are free to diverge when the store gains a column the API should not carry.
#[derive(Debug, Serialize, ToSchema)]
pub struct TombstoneDto {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// RFC 3339 timestamp of the deletion.
    pub deleted_at: String,
    /// The identity that deleted it, when the deletion carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<String>,
    /// RFC 3339 timestamp of when this tombstone's detail was stripped, or
    /// absent when it still has it. The field that tells a reader whether an
    /// absent `checksum` means "never recorded" or "aged out".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail_compacted_at: Option<String>,
    /// RFC 3339 timestamp of the original publish. Never stripped.
    pub published_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl From<Tombstone> for TombstoneDto {
    fn from(t: Tombstone) -> Self {
        Self {
            registry: t.registry,
            name: t.name,
            version: t.version,
            deleted_at: t.deleted_at.to_rfc3339(),
            deleted_by: t.deleted_by,
            detail_compacted_at: t.detail_compacted_at.map(|d| d.to_rfc3339()),
            published_at: t.published_at.to_rfc3339(),
            published_by: t.published_by,
            checksum: t.checksum,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TombstoneListResponse {
    pub registry: String,
    pub total: usize,
    pub tombstones: Vec<TombstoneDto>,
}

/// List the coordinates deleted from a local/hybrid registry (admin).
///
/// These version numbers are permanently spent: a publish to any of them is
/// refused, whoever makes it and however long ago the deletion was.
#[utoipa::path(
    get,
    path = "/api/v1/admin/registries/{registry}/tombstones",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        TombstoneQuery,
    ),
    responses(
        (status = 200, description = "Deleted coordinates, newest deletion first",
            body = TombstoneListResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/registries/{registry}/tombstones")]
pub async fn list_tombstones(
    path: web::Path<String>,
    query: web::Query<TombstoneQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();
    let tombstones = local_svc
        .backend
        .list_tombstones(&registry, query.name.as_deref())
        .await
        .map_err(AppError::from)?;
    let tombstones: Vec<TombstoneDto> = tombstones.into_iter().map(TombstoneDto::from).collect();
    Ok(HttpResponse::Ok().json(TombstoneListResponse {
        registry,
        total: tombstones.len(),
        tombstones,
    }))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct CompactQuery {
    /// Report without writing. Absent falls back to the registry's configured
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
pub struct CompactionResponse {
    pub registry: String,
    /// Tombstones whose detail was stripped, or would have been under `dry_run`.
    pub compacted: u64,
    /// Tombstones examined and left alone — inside the window, or already compacted.
    pub skipped: u64,
    /// True when nothing was written.
    pub dry_run: bool,
    /// The affected coordinates, `"{name}@{version}"`.
    pub coordinates: Vec<String>,
}

impl CompactionResponse {
    fn new(registry: String, report: CompactionReport) -> Self {
        Self {
            registry,
            compacted: report.compacted,
            skipped: report.skipped,
            dry_run: report.dry_run,
            coordinates: report.coordinates,
        }
    }
}

/// Strip aged-out tombstone detail in a local/hybrid registry (admin).
///
/// Compaction discards the checksum, publisher, signature and index metadata of
/// versions deleted longer ago than `[registries.retention]
/// tombstone_detail_for_days`. It never discards the coordinate claim, and there
/// is no endpoint, setting or query parameter that does: a compacted tombstone
/// still refuses a re-publish forever (RFC 0016 §4.5).
///
/// `409` when the registry has no `tombstone_detail_for_days` configured. Not a
/// silent no-op: an operator calling this on an unconfigured registry believes
/// they are reclaiming space, and a `200 { "compacted": 0 }` would confirm it.
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/tombstones/compact",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        CompactQuery,
    ),
    responses(
        (status = 200, description = "What was stripped, or would be under dry_run",
            body = CompactionResponse),
        (status = 403, description = "Admin role required"),
        (status = 409, description = "No tombstone_detail_for_days configured for this registry"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/tombstones/compact")]
pub async fn compact_tombstones(
    path: web::Path<String>,
    query: web::Query<CompactQuery>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let registry = path.into_inner();

    // Through the service's own lock rather than a separate `Data<HotConfigLock>`
    // extractor: it is the same `Arc`, and taking it from the service means the
    // policy this handler reads cannot be from a different reload than the one
    // the run underneath it will use.
    let policy = {
        let snapshot = local_svc.hot.read().await;
        snapshot.retention.get(&registry).cloned()
    };
    let Some(policy) = policy else {
        return Err(AppError::conflict(format!(
            "registry '{registry}' has no [registries.retention] tombstone_detail_for_days, so \
             every tombstone keeps its detail — which is the default. Nothing to compact."
        )));
    };
    let Some(window) = policy.tombstone_detail_for else {
        return Err(AppError::conflict(format!(
            "registry '{registry}' has a [registries.retention] block with no \
             tombstone_detail_for_days, so every tombstone keeps its detail. Nothing to compact."
        )));
    };

    // `||` rather than `unwrap_or`: the query may only ever make the run *more*
    // conservative. A configured dry_run = true is an operator's decision that a
    // request should not be able to reverse.
    let dry_run = policy.dry_run || query.dry_run.unwrap_or(false);

    let report = local_svc
        .compact_tombstone_detail(&registry, window, dry_run, &identity.0)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().json(CompactionResponse::new(registry, report)))
}
