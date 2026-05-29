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
    Denied {
        reason: String,
    },
    #[serde(rename = "error")]
    ProxyError {
        reason: String,
    },
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
    pub fn allowed_download(
        package_id: PackageId,
        user_id: Option<String>,
        user_role: Role,
    ) -> Self {
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

    pub fn proxy_error(
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
            result: AccessResult::ProxyError { reason },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::{PackageId, Role};

    fn pkg() -> PackageId {
        PackageId::new("cargo", "tokio", "1.0.0")
    }

    #[test]
    fn is_denied_only_for_denied_variant() {
        assert!(!AccessResult::Allowed.is_denied());
        assert!(AccessResult::Denied { reason: "blocked".into() }.is_denied());
        assert!(!AccessResult::ProxyError { reason: "timeout".into() }.is_denied());
    }

    #[test]
    fn allowed_download_sets_correct_fields() {
        let ev = AccessEvent::allowed_download(pkg(), Some("alice".into()), Role::User);
        assert!(matches!(ev.result, AccessResult::Allowed));
        assert!(matches!(ev.action, AccessAction::Download));
        assert_eq!(ev.user_id.as_deref(), Some("alice"));
        assert_eq!(ev.user_role, Role::User);
    }

    #[test]
    fn denied_download_sets_reason() {
        let ev =
            AccessEvent::denied_download(pkg(), None, Role::Anonymous, "blocklisted".into());
        assert!(
            matches!(&ev.result, AccessResult::Denied { reason } if reason == "blocklisted")
        );
    }

    #[test]
    fn proxy_error_sets_reason() {
        let ev =
            AccessEvent::proxy_error(pkg(), None, Role::Anonymous, "upstream timeout".into());
        assert!(
            matches!(&ev.result, AccessResult::ProxyError { reason } if reason == "upstream timeout")
        );
    }

    #[test]
    fn event_filter_new_default_limit() {
        let f = EventFilter::new();
        assert_eq!(f.limit, 100);
        assert_eq!(f.offset, 0);
        assert!(!f.denied_only);
        assert!(f.registry.is_none());
    }
}
