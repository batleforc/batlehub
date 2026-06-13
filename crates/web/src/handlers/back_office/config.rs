use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::entities::{BannerLevel, GlobalBanner};

use super::require_admin;
use crate::{
    error::AppError,
    extractors::AuthIdentity,
    services::{
        BannerService, ConfigChangeRow, ConfigReloadService, PendingReloadSnapshot, ReloadDiff,
    },
};

// ── Shared guards ─────────────────────────────────────────────────────────────

fn require_hot_reload(svc: &ConfigReloadService) -> Result<(), AppError> {
    if svc.hot_reload_enabled {
        Ok(())
    } else {
        Err(AppError::service_unavailable(
            "hot reload is disabled on this instance (BATLEHUB_DISABLE_HOT_RELOAD=1)",
        ))
    }
}

// ── Config reload ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ReloadResponse {
    pub diff: ReloadDiff,
}

/// Immediately reload the configuration (load, validate, and apply atomically).
#[utoipa::path(
    post,
    path = "/api/v1/admin/config/reload",
    tag = "back-office",
    responses(
        (status = 200, description = "Config reloaded", body = ReloadResponse),
        (status = 400, description = "Validation or probe failure"),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Hot reload disabled"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/config/reload")]
pub async fn reload_config(
    identity: AuthIdentity,
    reload_svc: web::Data<Arc<ConfigReloadService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    require_hot_reload(&reload_svc)?;
    let user_id = identity.0.user_id.as_deref().unwrap_or("unknown");
    let diff = reload_svc
        .reload_immediate(user_id)
        .await
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    Ok(web::Json(ReloadResponse { diff }))
}

/// Get the current pending reload (loaded by the file watcher or a previous request).
#[utoipa::path(
    get,
    path = "/api/v1/admin/config/pending",
    tag = "back-office",
    responses(
        (status = 200, description = "Pending reload snapshot", body = PendingReloadSnapshot),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "No pending reload"),
        (status = 503, description = "Hot reload disabled"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/config/pending")]
pub async fn get_pending_reload(
    identity: AuthIdentity,
    reload_svc: web::Data<Arc<ConfigReloadService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    require_hot_reload(&reload_svc)?;
    reload_svc
        .pending_snapshot()
        .map(web::Json)
        .ok_or_else(|| AppError::not_found("no pending reload"))
}

/// Apply the current pending reload.
#[utoipa::path(
    post,
    path = "/api/v1/admin/config/pending/apply",
    tag = "back-office",
    responses(
        (status = 200, description = "Pending reload applied", body = ReloadResponse),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "No pending reload"),
        (status = 409, description = "Pending reload expired"),
        (status = 503, description = "Hot reload disabled"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/config/pending/apply")]
pub async fn apply_pending_reload(
    identity: AuthIdentity,
    reload_svc: web::Data<Arc<ConfigReloadService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    require_hot_reload(&reload_svc)?;
    let user_id = identity.0.user_id.as_deref().unwrap_or("unknown");
    let diff = reload_svc.apply(user_id).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("no pending") {
            AppError::not_found(msg)
        } else if msg.contains("expired") {
            AppError::conflict(msg)
        } else {
            AppError::bad_request(msg)
        }
    })?;
    Ok(web::Json(ReloadResponse { diff }))
}

/// Discard the current pending reload without applying.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/config/pending",
    tag = "back-office",
    responses(
        (status = 204, description = "Pending reload discarded"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "No pending reload"),
        (status = 503, description = "Hot reload disabled"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/config/pending")]
pub async fn discard_pending_reload(
    identity: AuthIdentity,
    reload_svc: web::Data<Arc<ConfigReloadService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    require_hot_reload(&reload_svc)?;
    if reload_svc.discard_pending() {
        Ok(HttpResponse::NoContent().finish())
    } else {
        Err(AppError::not_found("no pending reload"))
    }
}

#[derive(Deserialize, ToSchema, IntoParams)]
pub struct ChangesQuery {
    #[serde(default)]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}
fn default_per_page() -> u64 {
    50
}

#[derive(Serialize, ToSchema)]
pub struct ConfigChangesResponse {
    pub items: Vec<ConfigChangeRow>,
    pub page: u64,
    pub per_page: u64,
}

/// List config change history.
#[utoipa::path(
    get,
    path = "/api/v1/admin/config/changes",
    tag = "back-office",
    params(ChangesQuery),
    responses(
        (status = 200, description = "Config change history", body = ConfigChangesResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/config/changes")]
pub async fn list_config_changes(
    identity: AuthIdentity,
    query: web::Query<ChangesQuery>,
    reload_svc: web::Data<Arc<ConfigReloadService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let items = reload_svc
        .list_changes(query.page, query.per_page)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(web::Json(ConfigChangesResponse {
        items,
        page: query.page,
        per_page: query.per_page,
    }))
}

// ── Banner endpoints ──────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SetBannerRequest {
    pub message: String,
    pub level: BannerLevel,
}

/// Set or replace the global admin banner.
#[utoipa::path(
    put,
    path = "/api/v1/admin/banner",
    tag = "back-office",
    request_body = SetBannerRequest,
    responses(
        (status = 200, description = "Banner set"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/banner")]
pub async fn set_banner(
    identity: AuthIdentity,
    body: web::Json<SetBannerRequest>,
    banner_svc: web::Data<Arc<BannerService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let set_by = identity
        .0
        .user_id
        .clone()
        .unwrap_or_else(|| "admin".to_owned());
    banner_svc
        .set(GlobalBanner {
            message: body.message.clone(),
            level: body.level.clone(),
            set_at: Utc::now(),
            set_by,
        })
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::Ok().finish())
}

/// Clear the global admin banner.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/banner",
    tag = "back-office",
    responses(
        (status = 204, description = "Banner cleared"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/banner")]
pub async fn clear_banner(
    identity: AuthIdentity,
    banner_svc: web::Data<Arc<BannerService>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    banner_svc.clear().await.map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_core::entities::{Identity, Role};

    fn admin_id() -> AuthIdentity {
        AuthIdentity(Identity {
            user_id: Some("admin".into()),
            role: Role::Admin,
            auth_provider: None,
            groups: vec![],
        })
    }

    fn user_id() -> AuthIdentity {
        AuthIdentity(Identity {
            user_id: Some("user".into()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        })
    }

    fn disabled_svc() -> Arc<ConfigReloadService> {
        use crate::services::HotConfigBuilder;
        use batlehub_core::services::new_hot_lock;
        use std::collections::HashMap;

        let hot = new_hot_lock(batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            ..Default::default()
        });
        let access = crate::new_access_lock(crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        });
        let builder: HotConfigBuilder =
            Arc::new(|_| anyhow::bail!("builder not used in this test"));
        Arc::new(ConfigReloadService::new(
            hot,
            access,
            crate::RegistryMap::new(HashMap::new()),
            crate::RegistryModeMap::new(HashMap::new()),
            crate::UpstreamMap::new(HashMap::new()),
            crate::CargoIndexMap::new(HashMap::new()),
            "config.toml".to_owned(),
            None,
            false, // hot_reload_enabled = false
            builder,
            None,
        ))
    }

    #[test]
    fn require_hot_reload_returns_503_when_disabled() {
        let svc = disabled_svc();
        let err = require_hot_reload(&svc).unwrap_err();
        assert_eq!(err.status, actix_web::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn require_hot_reload_passes_when_enabled() {
        use crate::services::HotConfigBuilder;
        use batlehub_core::services::new_hot_lock;
        use std::collections::HashMap;

        let hot = new_hot_lock(batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            ..Default::default()
        });
        let access = crate::new_access_lock(crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        });
        let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("unused"));
        let svc = Arc::new(ConfigReloadService::new(
            hot,
            access,
            crate::RegistryMap::new(HashMap::new()),
            crate::RegistryModeMap::new(HashMap::new()),
            crate::UpstreamMap::new(HashMap::new()),
            crate::CargoIndexMap::new(HashMap::new()),
            "config.toml".to_owned(),
            None,
            true,
            builder,
            None,
        ));
        assert!(require_hot_reload(&svc).is_ok());
    }

    #[test]
    fn require_admin_blocks_non_admin() {
        assert!(require_admin(&user_id()).is_err());
        assert!(require_admin(&admin_id()).is_ok());
    }
}
