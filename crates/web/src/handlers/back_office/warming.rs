use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::services::WarmingService;

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, RegistryMap};

/// Map of registry name → WarmingService, injected as app data.
pub type WarmingServiceMap = HashMap<String, Arc<WarmingService>>;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct WarmRequest {
    /// A package name to warm, optionally with a pinned version (`"lodash"` or
    /// `"lodash@4.17.21"`). Use for package-centric registries.
    #[serde(default)]
    pub package: Option<String>,
    /// Multiple package names to warm (same form as `package`).
    #[serde(default)]
    pub packages: Vec<String>,
    /// A single upstream artifact path to warm, for path-addressed registries
    /// (`deb`/`rpm`/`jetbrains`), e.g. `"idea/ideaIC-2024.1.4.tar.gz"`.
    #[serde(default)]
    pub path: Option<String>,
    /// Multiple upstream artifact paths to warm (same form as `path`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Override the number of most-recent versions to warm per package. Falls back
    /// to the registry's `warm_latest_n` config when absent.
    pub versions: Option<usize>,
}

#[derive(Serialize, ToSchema)]
pub struct WarmResponse {
    pub warmed: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Pre-warm cached artifacts for a specific package in a registry (admin).
#[utoipa::path(
    post,
    path = "/api/v1/admin/registries/{registry}/warm",
    tag = "back-office",
    params(
        ("registry" = String, Path, description = "Registry name"),
    ),
    request_body = WarmRequest,
    responses(
        (status = 200, description = "Warming completed", body = WarmResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Registry not found or warming not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/warm")]
pub async fn warm_registry(
    identity: AuthIdentity,
    path: web::Path<String>,
    body: web::Json<WarmRequest>,
    registry_map: web::Data<RegistryMap>,
    warming_map: web::Data<WarmingServiceMap>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;

    let registry = path.into_inner();

    if !registry_map.contains(&registry) {
        return Err(AppError::not_found("registry not found"));
    }

    let svc = warming_map
        .get(&registry)
        .ok_or_else(|| AppError::not_found("warming not configured for this registry"))?;

    // Gather packages (package + packages) and paths (path + paths).
    let mut packages = body.packages.clone();
    packages.extend(body.package.clone());
    let mut paths = body.paths.clone();
    paths.extend(body.path.clone());

    if packages.is_empty() && paths.is_empty() {
        return Err(AppError::bad_request(
            "specify at least one of: package, packages, path, paths".to_owned(),
        ));
    }

    // Version-based warming for package-centric registries (honour the optional
    // per-request version count); path-based warming for path-addressed registries.
    let pkg_report = if let Some(n) = body.versions {
        svc.with_latest_n(n).warm_all(&packages).await
    } else {
        svc.warm_all(&packages).await
    };
    let path_report = svc.warm_all_paths(&paths).await;

    Ok(web::Json(WarmResponse {
        warmed: pkg_report.warmed + path_report.warmed,
        skipped: pkg_report.skipped + path_report.skipped,
        errors: pkg_report.errors + path_report.errors,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractors::AuthIdentity;
    use batlehub_core::entities::{Identity, Role};

    fn id(role: Role) -> AuthIdentity {
        AuthIdentity(Identity {
            user_id: Some("u".into()),
            role,
            auth_provider: None,
            groups: vec![],
        })
    }

    #[test]
    fn require_admin_passes_for_admin() {
        assert!(require_admin(&id(Role::Admin)).is_ok());
    }

    #[test]
    fn require_admin_fails_for_non_admin() {
        assert!(require_admin(&id(Role::User)).is_err());
        assert!(require_admin(&id(Role::Anonymous)).is_err());
    }
}
