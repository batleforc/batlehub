use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entities::{PackageId, Role};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AccessAction {
    Download,
    ViewMetadata,
    Block,
    Unblock,
    Delete,
    /// A principal (user or group) was granted ownership of a package.
    AddOwner,
    /// A principal (user or group) had ownership of a package revoked.
    RemoveOwner,
    /// A package's visibility (public/internal/team) was changed.
    SetVisibility,
    /// An account-wide action: a user was blocked from authenticating.
    BlockUser,
    /// An account-wide action: a previously blocked user was unblocked.
    UnblockUser,
    /// A network-wide action: an IP address was blocked.
    BlockIp,
    /// A network-wide action: a previously blocked IP address was unblocked.
    UnblockIp,
    /// The access-audit trail itself was purged up to a cutoff timestamp.
    AuditPurge,
    /// A local/hybrid-mode version was yanked (hidden from install, still resolvable by exact pin).
    Yank,
    /// A previously yanked version was restored.
    Unyank,
    /// A local/hybrid-mode version was flagged deprecated.
    Deprecate,
    /// A deprecation was reversed.
    Undeprecate,
    /// A local/hybrid-mode version was hidden from registry-protocol listings.
    Unlist,
    /// An unlisted version was made visible in listings again.
    Relist,
    /// A principal was added to a registry's beta channel.
    AddBetaMember,
    /// A principal was removed from a registry's beta channel.
    RemoveBetaMember,
    /// A team namespace was claimed by a principal.
    ClaimNamespace,
    /// A previously claimed team namespace was released.
    ReleaseNamespace,
    /// A user's publish/download quota usage was reset by an admin.
    ResetQuota,
    /// A registry's aged-out tombstone detail was stripped (RFC 0016 §4.5).
    ///
    /// An action in its own right rather than a flavour of [`Self::Delete`],
    /// following the precedent [`Self::AuditPurge`] set: compaction is
    /// destructive to *history* while being harmless to the invariant, which is
    /// a different fact about a system than a version being deleted, and an
    /// operator reading the trail has to be able to separate the two.
    TombstoneCompact,
    /// A version was pinned against retention, or the pin was released
    /// (RFC 0016 §4.1).
    ///
    /// One action for both directions, unlike `Yank`/`Unyank`: a pin is a toggle
    /// whose current state is readable from the version row, and two actions
    /// would make "who exempted this version from the policy" a question you
    /// answer by scanning for the newest of two event kinds rather than the
    /// newest of one.
    SetRetentionPin,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AccessEvent {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub user_role: Role,
    /// The package coordinate this event is about, when applicable.
    ///
    /// `None` for account-wide/network-wide admin actions that are not scoped
    /// to a specific package (e.g. blocking a user or an IP address).
    pub package_id: Option<PackageId>,
    pub action: AccessAction,
    pub result: AccessResult,
    pub timestamp: DateTime<Utc>,
    /// Caller's IP address (from X-Forwarded-For / RemoteAddr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    /// HTTP User-Agent from the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
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
            package_id: Some(package_id),
            action: AccessAction::Download,
            result: AccessResult::Allowed,
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
        }
    }

    /// An allowed read of something that is *about* an artifact rather than the
    /// artifact — the counterpart of [`AccessEvent::denied_metadata`].
    ///
    /// Its one caller today is the checksum/signature sidecar split described on
    /// [`PackageId::is_verification_sidecar`]: the fetch is recorded, so the
    /// audit trail stays complete, but it does not count as a download.
    pub fn allowed_metadata(
        package_id: PackageId,
        user_id: Option<String>,
        user_role: Role,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            user_role,
            package_id: Some(package_id),
            action: AccessAction::ViewMetadata,
            result: AccessResult::Allowed,
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
        }
    }

    /// [`AccessEvent::allowed_download`], or [`AccessEvent::allowed_metadata`]
    /// when the coordinate names a checksum or signature sidecar.
    ///
    /// The two recording sites — `ProxyService::handle` and
    /// `LocalRegistryService::record_download` — must agree on this, or a
    /// hybrid registry's download counts would depend on whether the artifact
    /// happened to be published locally. Hence one function rather than the
    /// same `if` written twice.
    pub fn allowed_read(package_id: PackageId, user_id: Option<String>, user_role: Role) -> Self {
        if package_id.is_verification_sidecar() {
            Self::allowed_metadata(package_id, user_id, user_role)
        } else {
            Self::allowed_download(package_id, user_id, user_role)
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
            package_id: Some(package_id),
            action: AccessAction::Download,
            result: AccessResult::Denied { reason },
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
        }
    }

    /// A refused *listing* — a version document a client was not allowed to
    /// read.
    ///
    /// [`AccessAction::ViewMetadata`] rather than `Download`, because nothing
    /// was downloaded. Recording a listing as a download puts rows in the audit
    /// trail that transferred no bytes, which reads as traffic that never
    /// happened when an incident asks what an identity actually pulled.
    ///
    /// There is deliberately no `allowed_metadata` counterpart: an allowed
    /// listing is counted in `ProxyMetrics` and rolled up hourly, not filed per
    /// request. A `cargo build` over a 400-crate graph is 400 listings, and one
    /// row each would drown the trail in the least interesting of the three
    /// questions it answers. Denials are few and each one matters, so they
    /// stay rows.
    pub fn denied_metadata(
        package_id: PackageId,
        user_id: Option<String>,
        user_role: Role,
        reason: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id,
            user_role,
            package_id: Some(package_id),
            action: AccessAction::ViewMetadata,
            result: AccessResult::Denied { reason },
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
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
            package_id: Some(package_id),
            action: AccessAction::Download,
            result: AccessResult::ProxyError { reason },
            timestamp: Utc::now(),
            ip_address: None,
            user_agent: None,
        }
    }

    /// Builder: attach the caller's IP address and User-Agent to this event.
    pub fn with_ip_ua(mut self, ip: Option<String>, ua: Option<String>) -> Self {
        self.ip_address = ip;
        self.user_agent = ua;
        self
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
        assert!(AccessResult::Denied {
            reason: "blocked".into()
        }
        .is_denied());
        assert!(!AccessResult::ProxyError {
            reason: "timeout".into()
        }
        .is_denied());
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
        let ev = AccessEvent::denied_download(pkg(), None, Role::Anonymous, "blocklisted".into());
        assert!(matches!(&ev.result, AccessResult::Denied { reason } if reason == "blocklisted"));
    }

    /// A listing that transferred no bytes must not look like a download in the
    /// trail.
    #[test]
    fn denied_metadata_records_a_view_not_a_download() {
        let ev = AccessEvent::denied_metadata(pkg(), Some("bob".into()), Role::User, "rbac".into());
        assert!(matches!(ev.action, AccessAction::ViewMetadata));
        assert!(matches!(&ev.result, AccessResult::Denied { reason } if reason == "rbac"));
        assert_eq!(ev.user_id.as_deref(), Some("bob"));
    }

    #[test]
    fn proxy_error_sets_reason() {
        let ev = AccessEvent::proxy_error(pkg(), None, Role::Anonymous, "upstream timeout".into());
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
