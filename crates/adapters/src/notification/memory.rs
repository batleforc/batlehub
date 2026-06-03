use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use batlehub_core::{
    entities::{InboundWebhookEvent, NotificationEventType, NotificationSubscription},
    error::CoreError,
    ports::NotificationPort,
};

/// In-memory implementation of `NotificationPort`.
///
/// Suitable for tests and single-instance deployments where persistence is not required.
pub struct InMemoryNotificationStore {
    subscriptions: Arc<RwLock<Vec<NotificationSubscription>>>,
    inbound_events: Arc<RwLock<Vec<InboundWebhookEvent>>>,
}

impl Default for InMemoryNotificationStore {
    fn default() -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(Vec::new())),
            inbound_events: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl InMemoryNotificationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl NotificationPort for InMemoryNotificationStore {
    async fn add_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError> {
        self.subscriptions.write().await.push(sub);
        Ok(())
    }

    async fn list_subscriptions(&self) -> Result<Vec<NotificationSubscription>, CoreError> {
        Ok(self.subscriptions.read().await.clone())
    }

    async fn get_subscription(
        &self,
        id: Uuid,
    ) -> Result<Option<NotificationSubscription>, CoreError> {
        Ok(self
            .subscriptions
            .read()
            .await
            .iter()
            .find(|s| s.id == id)
            .cloned())
    }

    async fn update_subscription(&self, sub: NotificationSubscription) -> Result<(), CoreError> {
        let mut lock = self.subscriptions.write().await;
        let pos = lock
            .iter()
            .position(|s| s.id == sub.id)
            .ok_or_else(|| CoreError::NotFound(format!("subscription {}", sub.id)))?;
        lock[pos] = sub;
        Ok(())
    }

    async fn remove_subscription(&self, id: Uuid) -> Result<(), CoreError> {
        self.subscriptions.write().await.retain(|s| s.id != id);
        Ok(())
    }

    async fn get_matching_subscriptions(
        &self,
        registry: &str,
        package: &str,
        event_type: &NotificationEventType,
    ) -> Result<Vec<NotificationSubscription>, CoreError> {
        Ok(self
            .subscriptions
            .read()
            .await
            .iter()
            .filter(|s| s.matches(registry, package, event_type))
            .cloned()
            .collect())
    }

    async fn record_inbound_event(&self, event: InboundWebhookEvent) -> Result<(), CoreError> {
        self.inbound_events.write().await.push(event);
        Ok(())
    }

    async fn list_inbound_events(&self, limit: i64) -> Result<Vec<InboundWebhookEvent>, CoreError> {
        let lock = self.inbound_events.read().await;
        let limit = if limit <= 0 {
            lock.len()
        } else {
            limit as usize
        };
        Ok(lock.iter().rev().take(limit).cloned().collect())
    }
}
