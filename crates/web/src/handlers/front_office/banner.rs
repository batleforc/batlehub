use std::sync::Arc;

use actix_web::{get, web, Responder};

use batlehub_core::entities::GlobalBanner;

use crate::{error::AppError, services::BannerService};

/// Get the current global admin banner (unauthenticated).
///
/// Returns the banner if one is set, or `null` if none. Polled by the UI every 30 s.
#[utoipa::path(
    get,
    path = "/api/v1/banner",
    tag = "banner",
    responses(
        (status = 200, description = "Current banner or null", body = Option<GlobalBanner>),
    ),
)]
#[get("/api/v1/banner")]
pub async fn get_banner(
    banner_svc: web::Data<Arc<BannerService>>,
) -> Result<impl Responder, AppError> {
    let banner = banner_svc.get().await.map_err(AppError::from)?;
    Ok(web::Json(banner))
}
