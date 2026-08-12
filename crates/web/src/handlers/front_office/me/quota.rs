use std::sync::Arc;

use actix_web::{get, web, HttpResponse, Responder};
use serde::Serialize;
use utoipa::ToSchema;

use batlehub_core::services::{QuotaService, QuotaState, RegistryQuotaStatus};

use super::require_user_id;
use crate::{error::AppError, extractors::AuthIdentity};

/// How close the caller is to a registry's quota.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStateDto {
    /// Below the warning threshold.
    Ok,
    /// At or past `warn_threshold_pct` of a limit; publishing still succeeds.
    Warning,
    /// A limit is reached. Whether the next publish is refused depends on the
    /// registry's `enforcement` setting.
    AtLimit,
}

impl From<QuotaState> for QuotaStateDto {
    fn from(s: QuotaState) -> Self {
        match s {
            QuotaState::Ok => Self::Ok,
            QuotaState::Warning => Self::Warning,
            QuotaState::AtLimit => Self::AtLimit,
        }
    }
}

/// One registry's quota, as it applies to the caller.
///
/// A limit of `null` means that dimension is unlimited for this registry — the
/// other one is what the quota measures.
#[derive(Debug, Serialize, ToSchema)]
pub struct MyQuotaDto {
    pub registry: String,
    pub bytes_used: u64,
    pub bytes_limit: Option<u64>,
    /// This dimension's own verdict, or `null` when it has no limit. Present so
    /// a reader can see *which* threshold was crossed (RFC 0004 §4.2) — a
    /// registry whose versions are at 82% while its storage sits at 68% is not
    /// "at 82%" in both.
    pub bytes_state: Option<QuotaStateDto>,
    pub packages_used: u32,
    pub packages_limit: Option<u32>,
    /// This dimension's own verdict, or `null` when it has no limit.
    pub packages_state: Option<QuotaStateDto>,
    /// The percentage of a limit at which a `state` becomes `warning`.
    pub warn_threshold_pct: u8,
    /// The worse of the two dimensions — what the registry as a whole is at.
    pub state: QuotaStateDto,
}

impl From<RegistryQuotaStatus> for MyQuotaDto {
    fn from(s: RegistryQuotaStatus) -> Self {
        Self {
            registry: s.registry,
            bytes_used: s.bytes_used,
            bytes_limit: s.bytes_limit,
            bytes_state: s.bytes_state.map(Into::into),
            packages_used: s.packages_used,
            packages_limit: s.packages_limit,
            packages_state: s.packages_state.map(Into::into),
            warn_threshold_pct: s.warn_threshold_pct,
            state: s.state.into(),
        }
    }
}

/// The caller's own publish quota, per registry.
///
/// Only registries that both have a quota configured **and** are accessible to
/// the caller appear. A registry with no quota has nothing to measure, and one
/// the caller cannot reach is not theirs to learn about from here.
///
/// Takes no `user_id`: the subject is the token's. `/api/v1/admin/quota/…` is
/// the endpoint for reading someone else's, and it is admin-only for a reason
/// (RFC 0004 R3) — one endpoint serving two authorisation rules is how the
/// wrong row gets returned.
#[utoipa::path(
    get,
    path = "/api/v1/me/quota",
    tag = "user",
    responses(
        (status = 200, description = "The caller's usage against their limits, per quota-gated registry", body = Vec<MyQuotaDto>),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/me/quota")]
pub async fn my_quota(
    identity: AuthIdentity,
    quota_svc: web::Data<Arc<QuotaService>>,
    access: web::Data<crate::AccessConfigLock>,
) -> Result<impl Responder, AppError> {
    let user_id = require_user_id(&identity)?;

    let mut accessible: Vec<String> = access
        .read()
        .await
        .accessible_registries_for(&identity)
        .into_iter()
        .collect();
    accessible.sort();

    let rows = quota_svc
        .status_for_user(user_id, &accessible)
        .await
        .map_err(AppError::from)?;

    let dtos: Vec<MyQuotaDto> = rows.into_iter().map(Into::into).collect();
    Ok(HttpResponse::Ok().json(dtos))
}
