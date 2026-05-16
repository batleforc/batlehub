use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::{PackageId, Role};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccessAction {
    Download,
    ViewMetadata,
    Block,
    Unblock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum AccessResult {
    Allowed,
    Denied { reason: String },
}

impl AccessResult {
    pub fn is_denied(&self) -> bool {
        matches!(self, AccessResult::Denied { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessEvent {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub user_role: Role,
    pub package_id: PackageId,
    pub action: AccessAction,
    pub result: AccessResult,
    pub timestamp: DateTime<Utc>,
}

impl AccessEvent {
    pub fn allowed_download(package_id: PackageId, user_id: Option<String>, user_role: Role) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            user_role,
            package_id,
            action: AccessAction::Download,
            result: AccessResult::Allowed,
            timestamp: Utc::now(),
        }
    }

    pub fn denied_download(
        package_id: PackageId,
        user_id: Option<String>,
        user_role: Role,
        reason: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            user_role,
            package_id,
            action: AccessAction::Download,
            result: AccessResult::Denied { reason },
            timestamp: Utc::now(),
        }
    }
}

/// Filter for querying access events.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub registry: Option<String>,
    pub package_name: Option<String>,
    pub user_id: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub denied_only: bool,
    pub limit: u64,
    pub offset: u64,
}

impl EventFilter {
    pub fn new() -> Self {
        Self {
            limit: 100,
            ..Default::default()
        }
    }
}
