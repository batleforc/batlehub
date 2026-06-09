use std::sync::Arc;

use actix_web::{get, web, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use batlehub_core::{
    entities::{AccessAction, AccessResult, EventFilter, PackageFilter, PackageStatus},
    services::{AdminService, ProxyService},
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

// ── Package detail ────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailQuery {
    pub registry: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct PackageVersionDetail {
    pub id: Uuid,
    pub version: String,
    pub artifact: Option<String>,
    pub status: PackageStatusDetail,
    pub storage_key: String,
    pub cached: bool,
    /// Name of the storage backend holding this artifact (null if not yet cached or pre-migration).
    pub storage_backend: Option<String>,
    /// When the artifact was first stored in the cache (null if not yet cached or pre-migration).
    pub cached_at: Option<DateTime<Utc>>,
    pub access_count: u64,
    pub last_accessed: Option<DateTime<Utc>>,
    pub last_accessed_by: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PackageStatusDetail {
    Available,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: DateTime<Utc>,
    },
}

#[derive(Serialize, ToSchema)]
pub struct PackageEventDto {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub user_role: String,
    pub version: String,
    pub artifact: Option<String>,
    pub action: String,
    pub outcome: String,
    pub deny_reason: Option<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct PackageDetailResponse {
    pub registry: String,
    pub name: String,
    pub versions: Vec<PackageVersionDetail>,
    pub recent_events: Vec<PackageEventDto>,
}

/// Get detailed information about a specific package (all versions, access history, cache status).
#[utoipa::path(
    get,
    path = "/api/v1/admin/packages/detail",
    tag = "back-office",
    params(PackageDetailQuery),
    responses(
        (status = 200, description = "Package detail", body = PackageDetailResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/packages/detail")]
pub async fn package_detail(
    query: web::Query<PackageDetailQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
    pool: Option<web::Data<PgPool>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let filter = PackageFilter {
        registry: Some(query.registry.clone()),
        registries: vec![],
        name_exact: Some(query.name.clone()),
        name_contains: None,
        blocked_only: false,
        limit: 200,
        offset: 0,
    };
    let summaries = admin_svc
        .list_packages(filter)
        .await
        .map_err(AppError::from)?;

    let mut versions = Vec::with_capacity(summaries.len());
    for s in summaries {
        let storage_key = format!("artifact:{}", s.package_id.cache_key());
        let cached = proxy_svc
            .storage
            .exists(&storage_key)
            .await
            .unwrap_or(false);
        let (storage_backend, cached_at) = if let Some(ref p) = pool {
            let row = sqlx::query(
                "SELECT backend_name, stored_at FROM artifact_storage WHERE storage_key = $1",
            )
            .bind(&storage_key)
            .fetch_optional(p.get_ref())
            .await
            .ok()
            .flatten();
            let backend = row
                .as_ref()
                .and_then(|r| r.try_get::<String, _>("backend_name").ok());
            let at = row.and_then(|r| r.try_get::<DateTime<Utc>, _>("stored_at").ok());
            (backend, at)
        } else {
            (None, None)
        };
        let status = match s.status {
            PackageStatus::Available => PackageStatusDetail::Available,
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => PackageStatusDetail::Blocked {
                reason,
                blocked_by,
                blocked_at,
            },
        };
        versions.push(PackageVersionDetail {
            id: s.id,
            version: s.package_id.version,
            artifact: s.package_id.artifact,
            status,
            storage_key,
            cached,
            storage_backend,
            cached_at,
            access_count: s.access_count,
            last_accessed: s.last_accessed,
            last_accessed_by: s.last_accessed_by,
        });
    }

    let event_filter = EventFilter {
        registry: Some(query.registry.clone()),
        package_name: Some(query.name.clone()),
        user_id: None,
        from: None,
        to: None,
        denied_only: false,
        limit: 50,
        offset: 0,
    };
    let events = admin_svc
        .list_events(event_filter)
        .await
        .map_err(AppError::from)?;

    let recent_events = events
        .into_iter()
        .map(|e| {
            let (outcome, deny_reason) = match e.result {
                AccessResult::Allowed => ("allowed".to_string(), None),
                AccessResult::Denied { reason } => ("denied".to_string(), Some(reason)),
                AccessResult::ProxyError { reason } => ("error".to_string(), Some(reason)),
            };
            let action = match e.action {
                AccessAction::Download => "download",
                AccessAction::ViewMetadata => "view_metadata",
                AccessAction::Block => "block",
                AccessAction::Unblock => "unblock",
            };
            PackageEventDto {
                id: e.id,
                user_id: e.user_id,
                user_role: e.user_role.to_string(),
                version: e.package_id.version,
                artifact: e.package_id.artifact,
                action: action.to_string(),
                outcome,
                deny_reason,
                timestamp: e.timestamp,
            }
        })
        .collect();

    Ok(web::Json(PackageDetailResponse {
        registry: query.registry.clone(),
        name: query.name.clone(),
        versions,
        recent_events,
    }))
}
