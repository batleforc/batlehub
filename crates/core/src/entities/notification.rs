use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEventType {
    PackagePublished,
    PackageYanked,
    PackageUnyanked,
    PackageDeleted,
}

impl NotificationEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PackagePublished => "package_published",
            Self::PackageYanked => "package_yanked",
            Self::PackageUnyanked => "package_unyanked",
            Self::PackageDeleted => "package_deleted",
        }
    }
}

impl std::fmt::Display for NotificationEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for NotificationEventType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "package_published" => Ok(Self::PackagePublished),
            "package_yanked" => Ok(Self::PackageYanked),
            "package_unyanked" => Ok(Self::PackageUnyanked),
            "package_deleted" => Ok(Self::PackageDeleted),
            other => Err(format!("unknown event type: {other}")),
        }
    }
}

/// A package lifecycle event that may trigger outbound notifications.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotificationEvent {
    pub id: Uuid,
    pub event_type: NotificationEventType,
    pub registry: String,
    pub package_name: String,
    pub version: Option<String>,
    /// User ID of the actor who triggered the event.
    pub actor: String,
    pub occurred_at: DateTime<Utc>,
    /// Extra ecosystem-specific metadata (e.g. checksum, tags).
    pub metadata: serde_json::Value,
}

impl NotificationEvent {
    pub fn new(
        event_type: NotificationEventType,
        registry: impl Into<String>,
        package_name: impl Into<String>,
        version: Option<String>,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type,
            registry: registry.into(),
            package_name: package_name.into(),
            version,
            actor: actor.into(),
            occurred_at: Utc::now(),
            metadata: serde_json::Value::Object(Default::default()),
        }
    }
}

/// Admin-created subscription that routes matching events to a named channel.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NotificationSubscription {
    pub id: Uuid,
    /// When `None`, matches all registries.
    pub registry: Option<String>,
    /// When `None`, matches all packages in the selected registries.
    pub package_name: Option<String>,
    /// Event types that this subscription listens for.
    pub event_types: Vec<NotificationEventType>,
    /// Must match the `name` of a channel configured in `config.toml`.
    pub channel_name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

impl NotificationSubscription {
    pub fn matches(
        &self,
        registry: &str,
        package: &str,
        event_type: &NotificationEventType,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let registry_match = self.registry.as_deref().is_none_or(|r| r == registry);
        let package_match = self.package_name.as_deref().is_none_or(|p| p == package);
        let event_match = self.event_types.contains(event_type);
        registry_match && package_match && event_match
    }
}

/// A raw event received from an external system via the inbound webhook API.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InboundWebhookEvent {
    pub id: Uuid,
    pub webhook_name: String,
    pub payload: serde_json::Value,
    pub source_ip: Option<String>,
    pub received_at: DateTime<Utc>,
    /// `Some(true)` = HMAC verified, `Some(false)` = HMAC mismatch, `None` = no secret configured.
    pub signature_valid: Option<bool>,
}
