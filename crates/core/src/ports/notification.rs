use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    entities::{InboundWebhookEvent, NotificationEventType, NotificationSubscription},
    error::CoreError,
};

/// Storage port for notification subscriptions and inbound webhook events.
#[async_trait]
pub trait NotificationPort: Send + Sync {
    async fn add_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError>;
    async fn list_subscriptions(&self) -> Result<Vec<NotificationSubscription>, CoreError>;
    async fn get_subscription(
        &self,
        id: Uuid,
    ) -> Result<Option<NotificationSubscription>, CoreError>;
    async fn update_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError>;
    async fn remove_subscription(&self, id: Uuid) -> Result<(), CoreError>;

    /// Return all enabled subscriptions that match the given registry, package, and event type.
    async fn get_matching_subscriptions(
        &self,
        registry: &str,
        package: &str,
        event_type: &NotificationEventType,
    ) -> Result<Vec<NotificationSubscription>, CoreError>;

    async fn record_inbound_event(&self, event: InboundWebhookEvent) -> Result<(), CoreError>;
    async fn list_inbound_events(&self, limit: i64) -> Result<Vec<InboundWebhookEvent>, CoreError>;
}
