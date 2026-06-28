use std::sync::Arc;

use actix_web::http::header::{ContentDisposition, DispositionParam, DispositionType};
use actix_web::{delete, get, web, HttpResponse, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use batlehub_core::{entities::EventFilter, services::AdminService};

use super::require_admin;
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
    require_admin(&identity)?;

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

    let events = admin_svc
        .list_events(filter)
        .await
        .map_err(AppError::from)?;
    Ok(web::Json(events))
}

#[derive(Deserialize, IntoParams)]
pub struct ExportQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub registry: Option<String>,
    pub user_id: Option<String>,
    /// "json" (default) or "csv"
    #[serde(default)]
    pub format: String,
}

fn default_export_format(fmt: &str) -> &'static str {
    if fmt == "csv" {
        "csv"
    } else {
        "json"
    }
}

/// Export audit-log events for a time range (admin, SOC 2 compliance export).
#[utoipa::path(
    get,
    path = "/api/v1/admin/audit-log/export",
    tag = "back-office",
    params(ExportQuery),
    responses(
        (status = 200, description = "Audit log export (JSON or CSV)"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/audit-log/export")]
pub async fn export_audit_log(
    query: web::Query<ExportQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<HttpResponse, AppError> {
    require_admin(&identity)?;

    let filter = EventFilter {
        registry: query.registry.clone(),
        package_name: None,
        user_id: query.user_id.clone(),
        from: query.from,
        to: query.to,
        denied_only: false,
        limit: 100_000,
        offset: 0,
    };

    let events = admin_svc
        .list_events(filter)
        .await
        .map_err(AppError::from)?;

    let fmt = default_export_format(&query.format);
    let filename = format!("audit-log-{}.{fmt}", Utc::now().format("%Y%m%dT%H%M%SZ"));

    let disposition = ContentDisposition {
        disposition: DispositionType::Attachment,
        parameters: vec![DispositionParam::Filename(filename)],
    };

    if fmt == "csv" {
        let mut csv = String::from(
            "id,timestamp,user_id,user_role,registry,package_name,package_version,\
             package_artifact,action,outcome,deny_reason,ip_address,user_agent\n",
        );
        for e in &events {
            let deny_reason = match &e.result {
                batlehub_core::entities::AccessResult::Denied { reason } => reason.as_str(),
                batlehub_core::entities::AccessResult::ProxyError { reason } => reason.as_str(),
                _ => "",
            };
            let outcome = match &e.result {
                batlehub_core::entities::AccessResult::Allowed => "allowed",
                batlehub_core::entities::AccessResult::Denied { .. } => "denied",
                batlehub_core::entities::AccessResult::ProxyError { .. } => "error",
            };
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                e.id,
                e.timestamp.to_rfc3339(),
                e.user_id.as_deref().unwrap_or(""),
                e.user_role,
                e.package_id.registry,
                e.package_id.name,
                e.package_id.version,
                e.package_id.artifact.as_deref().unwrap_or(""),
                format!("{:?}", e.action).to_lowercase(),
                outcome,
                deny_reason,
                e.ip_address.as_deref().unwrap_or(""),
                e.user_agent.as_deref().unwrap_or(""),
            ));
        }
        Ok(HttpResponse::Ok()
            .insert_header(disposition)
            .content_type("text/csv")
            .body(csv))
    } else {
        let body = serde_json::to_string(&events)
            .map_err(|e| AppError::internal(format!("serialize: {e}")))?;
        Ok(HttpResponse::Ok()
            .insert_header(disposition)
            .content_type("application/json")
            .body(body))
    }
}

#[derive(Deserialize, IntoParams)]
pub struct PurgeQuery {
    pub before: DateTime<Utc>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PurgeResponse {
    pub deleted: u64,
}

/// Purge access-event rows older than `before` (admin).
#[utoipa::path(
    delete,
    path = "/api/v1/admin/audit-log",
    tag = "back-office",
    params(PurgeQuery),
    responses(
        (status = 200, description = "Number of rows deleted", body = PurgeResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/audit-log")]
pub async fn purge_audit_log(
    query: web::Query<PurgeQuery>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let deleted = admin_svc
        .purge_events_before(query.before)
        .await
        .map_err(AppError::from)?;
    Ok(web::Json(PurgeResponse { deleted }))
}
