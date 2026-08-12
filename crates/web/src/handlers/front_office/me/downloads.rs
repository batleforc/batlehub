use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{entities::PackageId, services::AdminService};

use super::require_user_id;
use crate::{error::AppError, extractors::AuthIdentity};

/// How far back `GET /api/v1/me/downloads` looks when `?days=` is absent.
const DEFAULT_WINDOW_DAYS: i64 = 30;
/// Hard ceiling on `?days=`, so one request cannot scan the whole audit trail.
const MAX_WINDOW_DAYS: i64 = 365;
const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;

#[derive(Debug, Deserialize, IntoParams)]
pub struct MyDownloadsQuery {
    /// Maximum rows to return. Clamped to 1–200; defaults to 50.
    pub limit: Option<u64>,
    /// How many days back to look. Clamped to 1–365; defaults to 30.
    pub days: Option<i64>,
}

/// One artifact the caller downloaded.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyDownloadDto {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// The sub-artifact within the coordinate, where the registry has one
    /// (a GitHub asset id, `tarball`, `vsix`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    pub downloaded_at: DateTime<Utc>,
}

/// What the caller has downloaded, most recent first.
///
/// Only this caller's own rows, only successful downloads. The filter is in
/// `PackageRepository::list_own_downloads` rather than assembled here, so no
/// future caller of that port can forget it (RFC 0004 §6.2).
///
/// Takes no `user_id`. `/api/v1/admin/audit-log` is the endpoint for reading
/// anyone else's, and it is admin-only.
#[utoipa::path(
    get,
    path = "/api/v1/me/downloads",
    tag = "user",
    params(MyDownloadsQuery),
    responses(
        (status = 200, description = "The caller's recent downloads, newest first", body = Vec<MyDownloadDto>),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me/downloads")]
pub async fn my_downloads(
    identity: AuthIdentity,
    query: web::Query<MyDownloadsQuery>,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    let user_id = require_user_id(&identity)?;

    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let days = query
        .days
        .unwrap_or(DEFAULT_WINDOW_DAYS)
        .clamp(1, MAX_WINDOW_DAYS);
    let since = Utc::now() - Duration::days(days);

    let events = admin_svc
        .list_own_downloads(user_id, since, limit)
        .await
        .map_err(AppError::from)?;

    let rows: Vec<MyDownloadDto> = events
        .into_iter()
        .filter_map(|e| {
            let PackageId {
                registry,
                name,
                version,
                artifact,
            } = e.package_id?;
            Some(MyDownloadDto {
                registry,
                name,
                version,
                artifact,
                downloaded_at: e.timestamp,
            })
        })
        .collect();

    Ok(HttpResponse::Ok().json(rows))
}
