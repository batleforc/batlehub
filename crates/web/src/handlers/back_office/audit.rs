use std::sync::Arc;

use actix_web::{Responder, get, web};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use batlehub_core::{
    entities::{EventFilter, Role},
    services::AdminService,
};

use crate::{error::AppError, extractors::AuthIdentity};

#[derive(Deserialize, IntoParams)]
pub struct AuditQuery {
    pub registry: Option<String>,
    pub user_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub denied_only: Option<bool>,
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

fn default_per_page() -> u64 {
    100
}

/// Query the access audit log (admin).
#[utoipa::path(
    get,
    path = "/api/v1/admin/audit-log",
    tag = "back-office",
    params(AuditQuery),
    responses(
        (status = 200, description = "Paginated access events"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/audit-log")]
pub async fn audit_log(
    query: web::Query<AuditQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    if identity.role != Role::Admin {
        return Err(AppError::forbidden("admin role required"));
    }

    let filter = EventFilter {
        registry: query.registry.clone(),
        package_name: None,
        user_id: query.user_id.clone(),
        from: query.from,
        to: query.to,
        denied_only: query.denied_only.unwrap_or(false),
        limit: query.per_page,
        offset: query.page * query.per_page,
    };

    let events = admin_svc.list_events(filter).await.map_err(AppError::from)?;
    Ok(web::Json(events))
}
