use std::collections::HashMap;
use std::sync::Arc;

use actix_web::{get, post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::services::WarmingService;

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
    /// (`deb`/`rpm`/`jetbrains`), e.g. `"idea/idea-2026.1.3.tar.gz"`.
    #[serde(default)]
    pub path: Option<String>,
    /// Multiple upstream artifact paths to warm (same form as `path`).
    #[serde(default)]
    pub paths: Vec<String>,
    /// Override the number of most-recent versions to warm per package. Falls back
    /// to the registry's `warm_latest_n` config when absent.
    pub versions: Option<usize>,
}

/// One package or path that did not warm (RFC 0004-bis A3).
#[derive(Serialize, ToSchema)]
pub struct WarmFailureDto {
    /// Package name, or the upstream path for a path-addressed registry.
    pub package: String,
    /// The version that failed. Absent when listing the versions is what failed.
    pub version: Option<String>,
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct WarmResponse {
    pub warmed: usize,
    pub skipped: usize,
    pub errors: usize,
    /// One entry per named failure, as the bulk endpoints already return.
    ///
    /// `errors` remains the count: a warming task that panics is counted here
    /// and cannot be named, so `failures.len() <= errors`. Reporting only the
    /// count left an operator with "3 errors" over eleven registries and no way
    /// to learn which three without reading the server log.
    pub failures: Vec<WarmFailureDto>,
}

#[derive(Serialize, ToSchema)]
pub struct WarmableRegistry {
    pub name: String,
    pub latest_n: usize,
    pub concurrency: usize,
}

#[derive(Serialize, ToSchema)]
pub struct WarmingStatusResponse {
    pub registries: Vec<WarmableRegistry>,
}

/// List registries that have warming configured and their settings.
#[utoipa::path(
    get,
    path = "/api/v1/admin/warming",
    tag = "back-office",
    responses(
        (status = 200, description = "Warmable registries", body = WarmingStatusResponse),
        (status = 403, description = "`cache:warm` required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/warming")]
pub async fn get_warming_status(
    identity: AuthIdentity,
    warming_map: web::Data<WarmingServiceMap>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::CacheWarm,
        None,
        &hot,
    )
    .await?;
    let mut registries: Vec<WarmableRegistry> = warming_map
        .iter()
        .map(|(name, svc)| WarmableRegistry {
            name: name.clone(),
            latest_n: svc.latest_n,
            concurrency: svc.concurrency,
        })
        .collect();
    registries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(web::Json(WarmingStatusResponse { registries }))
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
        (status = 403, description = "`cache:warm` required"),
        (status = 404, description = "Registry not found or warming not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/registries/{registry}/warm")]
pub async fn warm_registry(
    path: web::Path<String>,
    identity: AuthIdentity,
    body: web::Json<WarmRequest>,
    registry_map: web::Data<RegistryMap>,
    warming_map: web::Data<WarmingServiceMap>,
    hot: web::Data<batlehub_core::services::hot_config::HotConfigLock>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    crate::handlers::back_office::require_verb(
        &identity,
        batlehub_core::entities::Action::CacheWarm,
        Some(&registry),
        &hot,
    )
    .await?;

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
        failures: pkg_report
            .failures
            .into_iter()
            .chain(path_report.failures)
            .map(|f| WarmFailureDto {
                package: f.package,
                version: f.version,
                error: f.error,
            })
            .collect(),
    }))
}
