pub mod audit;
pub mod beta_channel;
pub mod bulk;
pub mod config;
pub mod health;
pub mod ip_blocks;
pub mod ownership;
pub mod packages;
pub mod quota;
pub mod stats;
pub mod team_namespaces;
pub mod visibility;
pub mod warming;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::{error::AppError, extractors::AuthIdentity};
use batlehub_core::entities::Role;

pub(super) fn require_admin(identity: &AuthIdentity) -> Result<(), AppError> {
    if identity.role != Role::Admin {
        Err(AppError::forbidden("admin role required"))
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
