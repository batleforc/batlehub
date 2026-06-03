use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;

use batlehub_core::{entities::SbomFormat, services::SbomService};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

fn default_sbom_format() -> String {
    "spdx".to_owned()
}

// ── Per-artifact SBOM ─────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct SbomQuery {
    /// SBOM format: `spdx` (default) or `cyclonedx`.
    #[serde(default = "default_sbom_format")]
    pub format: String,
}

/// Retrieve the SBOM for a specific artifact version.
#[utoipa::path(
    get,
    path = "/api/v1/sbom/{registry}/{name}/{version}",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("name"     = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Package version"),
        SbomQuery,
    ),
    responses(
        (status = 200, description = "SBOM document (JSON)"),
        (status = 400, description = "Unknown format"),
        (status = 403, description = "Authentication required"),
        (status = 404, description = "No SBOM found for this artifact"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/sbom/{registry}/{name}/{version}")]
pub async fn get_artifact_sbom(
    path: web::Path<(String, String, String)>,
    query: web::Query<SbomQuery>,
    identity: AuthIdentity,
    sbom_svc: web::Data<Arc<SbomService>>,
) -> Result<impl Responder, AppError> {
    // Requires at least an authenticated user (not anonymous).
    if identity.role == batlehub_core::entities::Role::Anonymous {
        return Err(AppError::forbidden("authentication required to access SBOMs"));
    }

    let (registry, name, version) = path.into_inner();

    let format = SbomFormat::parse(&query.format)
        .ok_or_else(|| AppError::bad_request(format!("unknown SBOM format '{}'", query.format)))?;

    // Try proxy artifact key first, then local registry key.
    let proxy_key = format!("artifact:{registry}/{name}/{version}");
    let local_key = format!("local:{registry}/{name}/{version}");

    let sbom = sbom_svc
        .get_artifact_sbom(&proxy_key, &format)
        .await
        .map_err(AppError::from)?;

    let sbom = if sbom.is_none() {
        sbom_svc
            .get_artifact_sbom(&local_key, &format)
            .await
            .map_err(AppError::from)?
    } else {
        sbom
    };

    match sbom {
        Some(s) => Ok(HttpResponse::Ok()
            .content_type("application/json")
            .json(s.document)),
        None => Err(AppError::not_found(format!(
            "no SBOM found for {registry}/{name}/{version} (format: {})",
            query.format
        ))),
    }
}

// ── Org-level SBOM export ─────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct SbomExportQuery {
    /// Filter to a specific registry name.
    pub registry: Option<String>,
    /// Earliest artifact creation timestamp (inclusive).
    pub from: Option<DateTime<Utc>>,
    /// Latest artifact creation timestamp (inclusive).
    pub to: Option<DateTime<Utc>>,
    /// SBOM format: `spdx` (default) or `cyclonedx`.
    #[serde(default = "default_sbom_format")]
    pub format: String,
}

/// Export an org-level SBOM covering all artifacts served in a time range.
#[utoipa::path(
    get,
    path = "/api/v1/sbom/export",
    tag = "back-office",
    params(SbomExportQuery),
    responses(
        (status = 200, description = "Merged SBOM document (JSON attachment)"),
        (status = 400, description = "Unknown format"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/sbom/export")]
pub async fn export_org_sbom(
    query: web::Query<SbomExportQuery>,
    identity: AuthIdentity,
    sbom_svc: web::Data<Arc<SbomService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let format = SbomFormat::parse(&query.format)
        .ok_or_else(|| AppError::bad_request(format!("unknown SBOM format '{}'", query.format)))?;

    let ext = match format {
        SbomFormat::Spdx => "spdx.json",
        SbomFormat::CycloneDx => "cyclonedx.json",
    };

    let ts = Utc::now().format("%Y%m%d%H%M%S");
    let registry_label = query.registry.as_deref().unwrap_or("all");
    let filename = format!("sbom-export-{registry_label}-{ts}.{ext}");

    let document = sbom_svc
        .export_org_sbom(
            query.registry.as_deref(),
            query.from,
            query.to,
            &format,
        )
        .await
        .map_err(AppError::from)?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        ))
        .json(document))
}
