use std::sync::Arc;

use actix_web::{delete, post, web, HttpResponse, Responder};
use sha2::{Digest, Sha256};

use batlehub_core::{
    entities::NotificationEventType,
    services::{LocalRegistryService, PublishRequest},
};

use crate::handlers::proxy::common::{
    collect_payload, dispatch_notification, publish_and_respond, require_local_mode,
    require_registry_type,
};
use crate::{
    error::AppError, extractors::AuthIdentity, services::NotificationService, RegistryMap,
    RegistryModeMap,
};

// ── Publish (local/hybrid only) ───────────────────────────────────────────────

/// Upload a Composer package ZIP (local/hybrid registries only).
///
/// The request body must be a ZIP file containing a `composer.json` at the root
/// or in a single top-level directory (GitHub-style zipball layout).
/// An optional `?version=x.y.z` query parameter overrides the version in `composer.json`.
#[utoipa::path(
    post,
    path = "/proxy/{registry}/api/upload",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("version"  = String, Query, description = "Version override (optional)"),
    ),
    responses(
        (status = 200, description = "Package published successfully"),
        (status = 403, description = "Access denied or quota exceeded"),
        (status = 409, description = "Version already exists"),
        (status = 422, description = "Invalid ZIP or versioning policy violation"),
    ),
    security(("bearer_token" = [])),
)]
#[allow(clippy::too_many_arguments)]
#[post("/proxy/{registry}/api/upload")]
pub async fn composer_upload(
    path: web::Path<String>,
    query: web::Query<UploadQuery>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "composer", &map)?;
    require_local_mode(&registry, &mode_map)?;

    // Validate the version override early before buffering the payload.
    if let Some(ref ver) = query.version {
        validate_version_param(ver)?;
    }

    let data = collect_payload(payload).await?;

    let meta =
        batlehub_adapters::registry::composer::parse_composer_zip(&data, query.version.as_deref())
            .map_err(|e| AppError::unprocessable(e.to_string()))?;

    let checksum = hex::encode(Sha256::digest(&data));

    let index_metadata = meta.composer_json.clone();

    let name = meta.name.clone();
    let version = meta.version.clone();

    publish_and_respond(
        &local_svc,
        &notification_svc,
        PublishRequest {
            registry,
            name: name.clone(),
            version: version.clone(),
            artifact: data,
            checksum,
            index_metadata,
            publisher: identity.0,
            signature_bytes: None,
            signature_type: None,
        },
        actix_web::http::StatusCode::OK,
        serde_json::json!({
            "status": "success",
            "name": name,
            "version": version,
        }),
    )
    .await
}

#[derive(serde::Deserialize)]
struct UploadQuery {
    version: Option<String>,
}

// ── Yank (local/hybrid only) ──────────────────────────────────────────────────

/// Yank a Composer package version (local/hybrid registries only).
#[utoipa::path(
    delete,
    path = "/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}",
    tag = "proxy/composer",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("vendor"   = String, Path, description = "Vendor name"),
        ("package"  = String, Path, description = "Package name"),
        ("version"  = String, Path, description = "Version to yank"),
    ),
    responses(
        (status = 200, description = "Version yanked"),
        (status = 403, description = "Access denied"),
        (status = 404, description = "Package or version not found"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/proxy/{registry}/api/packages/{vendor}/{package}/versions/{version}")]
pub async fn composer_yank(
    path: web::Path<(String, String, String, String)>,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    let (registry, vendor, package, version) = path.into_inner();
    require_registry_type(&registry, "composer", &map)?;
    require_local_mode(&registry, &mode_map)?;

    let name = format!("{vendor}/{package}");
    let actor = identity.0.user_id.clone().unwrap_or_default();
    local_svc
        .yank(&registry, &name, &version, &identity.0)
        .await
        .map_err(AppError::from)?;

    dispatch_notification(
        &notification_svc,
        NotificationEventType::PackageYanked,
        &registry,
        &name,
        Some(version.clone()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "message": format!("Successfully yanked {name} ({version})")
    })))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Validate that a `?version=` query-parameter value is safe to use as a
/// package version string. Rejects values that are excessively long or
/// contain characters outside the set allowed by Composer.
fn validate_version_param(v: &str) -> Result<(), AppError> {
    if v.len() > 128 {
        return Err(AppError::unprocessable(
            "version parameter too long".to_owned(),
        ));
    }
    let ok = v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '~' | '_' | 'v' | 'V'));
    if !ok {
        return Err(AppError::unprocessable(format!(
            "invalid characters in version parameter: '{v}'"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_version_param_accepts_valid() {
        assert!(validate_version_param("1.0.0").is_ok());
        assert!(validate_version_param("v2.3.4-beta.1").is_ok());
        assert!(validate_version_param("dev-main").is_ok());
    }

    #[test]
    fn validate_version_param_rejects_long() {
        let long = "a".repeat(129);
        assert!(validate_version_param(&long).is_err());
    }

    #[test]
    fn validate_version_param_rejects_special_chars() {
        assert!(validate_version_param("../../etc/passwd").is_err());
        assert!(validate_version_param("1.0.0; rm -rf /").is_err());
    }
}
