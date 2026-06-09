pub mod subscriptions;

pub use subscriptions::{
    create_subscription, delete_subscription, get_subscription, list_notification_channels,
    list_subscriptions, test_subscription, update_subscription,
};

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use batlehub_core::entities::NotificationEventType;

use crate::{error::AppError, services::NotificationService};

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSubscriptionRequest {
    /// When `null`, applies to all registries.
    pub registry: Option<String>,
    /// When `null`, applies to all packages in the selected registries.
    pub package_name: Option<String>,
    pub event_types: Vec<NotificationEventType>,
    pub channel_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSubscriptionRequest {
    pub registry: Option<String>,
    pub package_name: Option<String>,
    pub event_types: Vec<NotificationEventType>,
    pub channel_name: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelListResponse {
    pub channels: Vec<ChannelInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelInfo {
    pub name: String,
}

// ── Guard ─────────────────────────────────────────────────────────────────────

pub(super) fn require_notifications(
    svc: &Option<Arc<NotificationService>>,
) -> Result<&Arc<NotificationService>, AppError> {
    svc.as_ref()
        .ok_or_else(|| AppError::service_unavailable("notifications not configured"))
}
