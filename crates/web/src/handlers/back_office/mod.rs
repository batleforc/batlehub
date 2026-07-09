pub mod access_check;
pub mod audit;
pub mod bulk;
pub mod config;
pub mod explore;
pub mod governance;
pub mod health;
pub mod notification;
pub mod ops;
pub mod packages;
pub mod sbom;
pub mod stats;
pub mod visibility;

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
