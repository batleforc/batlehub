pub mod access_check;
pub mod audit;
pub mod beta_channel;
pub mod bulk;
pub mod config;
pub mod eviction;
pub mod explore;
pub mod health;
pub mod ip_blocks;
pub mod notification;
pub mod ownership;
pub mod packages;
pub mod quota;
pub mod sbom;
pub mod stats;
pub mod team_namespaces;
pub mod user_block;
pub mod visibility;
pub mod warming;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{error::AppError, extractors::AuthIdentity};
use batlehub_core::entities::Role;

pub(crate) fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
    } else {
        Ok(())
    }
}

/// Reject anonymous callers, without requiring the stronger `require_admin`
/// threshold. Shared by handlers that only need "authenticated at all" —
/// e.g. deb/rpm repo publish, per-artifact SBOM reads.
pub(crate) fn require_authenticated(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role == Role::Anonymous {
        Err(AppError::forbidden("authentication required"))
    } else {
        Ok(())
    }
}

pub(super) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
