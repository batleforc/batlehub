use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::{HttpResponse, Responder, delete, get, post, web};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::{entities::Role, ports::{BlockedIpInfo, IpBlockStore}};

use crate::{error::AppError, extractors::AuthIdentity};

fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BlockedIpDto {
    pub ip: String,
    pub blocked_at: u64,
    pub unblock_at: u64,
    pub reason: String,
}

impl From<BlockedIpInfo> for BlockedIpDto {
    fn from(b: BlockedIpInfo) -> Self {
        Self {
            ip: b.ip,
            blocked_at: b.blocked_at,
            unblock_at: b.unblock_at,
            reason: b.reason,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BlockIpRequest {
    pub ip: String,
    #[serde(default)]
    pub reason: Option<String>,
    /// Duration to block the IP in seconds (defaults to 3600).
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

/// List all currently blocked IPs.
#[utoipa::path(
    get,
    path = "/api/v1/admin/ip-blocks",
    tag = "back-office",
    responses(
        (status = 200, description = "List of blocked IPs"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/ip-blocks")]
pub async fn list_blocked_ips(
    identity: AuthIdentity,
    store: web::Data<Arc<dyn IpBlockStore>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let blocked: Vec<BlockedIpDto> = store
        .list_blocked()
        .await
        .map_err(AppError::from)?
        .into_iter()
        .map(BlockedIpDto::from)
        .collect();
    Ok(HttpResponse::Ok().json(blocked))
}

/// Manually block an IP address.
#[utoipa::path(
    post,
    path = "/api/v1/admin/ip-blocks",
    tag = "back-office",
    request_body = BlockIpRequest,
    responses(
        (status = 204, description = "IP blocked"),
        (status = 400, description = "Invalid IP address or duration"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/ip-blocks")]
pub async fn block_ip(
    body: web::Json<BlockIpRequest>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn IpBlockStore>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    IpAddr::from_str(&body.ip)
        .map_err(|_| AppError::bad_request(format!("'{}' is not a valid IP address", body.ip)))?;
    let duration = body.duration_secs.unwrap_or(3600);
    if duration == 0 {
        return Err(AppError::bad_request("duration_secs must be greater than 0"));
    }
    let unblock_at = now_unix() + duration;
    let reason = body.reason.as_deref().unwrap_or("manual");
    store
        .block_ip(&body.ip, unblock_at, reason)
        .await
        .map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}

/// Unblock an IP address.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/ip-blocks/{ip}",
    tag = "back-office",
    params(("ip" = String, Path, description = "IP address to unblock")),
    responses(
        (status = 204, description = "IP unblocked"),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/ip-blocks/{ip}")]
pub async fn unblock_ip(
    path: web::Path<(String,)>,
    identity: AuthIdentity,
    store: web::Data<Arc<dyn IpBlockStore>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let (ip,) = path.into_inner();
    store.unblock_ip(&ip).await.map_err(AppError::from)?;
    Ok(HttpResponse::NoContent().finish())
}
