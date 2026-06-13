use std::sync::Arc;

use batlehub_core::entities::{NotificationEvent, NotificationSubscription};
use uuid::Uuid;

use super::NotificationService;

impl NotificationService {
    /// Query matching subscriptions and spawn a background task per subscription.
    /// Never blocks the caller; errors are logged as warnings.
    /// The spawned task is tracked so `shutdown()` can await its completion.
    /// Drops the event (with a warning) if the concurrency cap is already reached.
    pub fn dispatch_event_background(self: &Arc<Self>, event: NotificationEvent) {
        let permit = match Arc::clone(&self.dispatch_semaphore).try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    event_type = %event.event_type,
                    registry   = %event.registry,
                    "notification: dispatch semaphore full ({} in-flight), dropping event",
                    super::MAX_CONCURRENT_DISPATCHES
                );
                return;
            }
        };
        // Lock BEFORE spawning so shutdown() cannot drain the vec in the gap
        // between tokio::spawn() returning a handle and the handle being pushed.
        let mut guard = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        // Prune completed handles to prevent unbounded growth.
        guard.retain(|h| !h.is_finished());
        let svc = Arc::clone(self);
        let handle = tokio::spawn(async move {
            let _permit = permit;
            svc.dispatch_event(event).await;
        });
        guard.push(handle);
    }

    /// Await all in-flight background dispatch tasks. Call this during graceful shutdown
    /// before the tokio runtime exits, so no in-flight notifications are dropped.
    pub async fn shutdown(&self) {
        let handles = {
            let mut g = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *g)
        };
        for handle in handles {
            let _ = handle.await;
        }
    }

    pub async fn dispatch_event(&self, event: NotificationEvent) {
        let subs = match self
            .store
            .get_matching_subscriptions(&event.registry, &event.package_name, &event.event_type)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("notification: failed to query subscriptions: {e}");
                return;
            }
        };

        for sub in subs {
            let Some(dispatcher) = self.channels.get(&sub.channel_name) else {
                tracing::warn!(
                    channel = %sub.channel_name,
                    "notification: subscription {} references unknown channel",
                    sub.id
                );
                continue;
            };
            if let Err(e) = dispatcher.dispatch(&event).await {
                tracing::warn!(channel = %sub.channel_name, "notification: dispatch failed: {e}");
            }
        }
    }

    /// Send a test notification to the channel used by the given subscription.
    pub async fn test_subscription(&self, sub: &NotificationSubscription) -> anyhow::Result<()> {
        let dispatcher = self
            .channels
            .get(&sub.channel_name)
            .ok_or_else(|| anyhow::anyhow!("channel '{}' not configured", sub.channel_name))?;
        dispatcher.send_test().await
    }

    /// List the names and types of all configured channels (no secrets exposed).
    pub fn channel_names(&self) -> Vec<String> {
        self.channels.keys().cloned().collect()
    }

    pub fn store(&self) -> &Arc<dyn batlehub_core::ports::NotificationPort> {
        &self.store
    }

    // ── Subscription store passthrough (used by handlers) ────────────────────────

    pub async fn add_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> Result<(), batlehub_core::error::CoreError> {
        self.store.add_subscription(sub).await
    }

    pub async fn list_subscriptions(
        &self,
    ) -> Result<Vec<NotificationSubscription>, batlehub_core::error::CoreError> {
        self.store.list_subscriptions().await
    }

    pub async fn get_subscription(
        &self,
        id: Uuid,
    ) -> Result<Option<NotificationSubscription>, batlehub_core::error::CoreError> {
        self.store.get_subscription(id).await
    }

    pub async fn update_subscription(
        &self,
        sub: NotificationSubscription,
    ) -> Result<(), batlehub_core::error::CoreError> {
        self.store.update_subscription(sub).await
    }

    pub async fn remove_subscription(
        &self,
        id: Uuid,
    ) -> Result<(), batlehub_core::error::CoreError> {
        self.store.remove_subscription(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use batlehub_adapters::notification::InMemoryNotificationStore;
    use batlehub_config::schema::{
        NotificationChannelConfig, NotificationsConfig, WebhookChannelConfig,
    };
    use batlehub_core::entities::NotificationEventType;

    fn webhook_config(name: &str, url: &str) -> NotificationsConfig {
        NotificationsConfig {
            enabled: true,
            channels: vec![NotificationChannelConfig::Webhook(WebhookChannelConfig {
                name: name.to_owned(),
                url: url.to_owned(),
                secret: None,
                timeout_secs: 5,
            })],
            inbound: vec![],
        }
    }

    fn make_service(config: &NotificationsConfig) -> Arc<NotificationService> {
        Arc::new(NotificationService::new(
            Arc::new(InMemoryNotificationStore::new()),
            config,
        ))
    }

    fn sample_subscription(channel_name: &str) -> NotificationSubscription {
        NotificationSubscription {
            id: Uuid::new_v4(),
            registry: None,
            package_name: None,
            event_types: vec![NotificationEventType::PackagePublished],
            channel_name: channel_name.to_owned(),
            created_by: "tester".to_owned(),
            created_at: chrono::Utc::now(),
            enabled: true,
        }
    }

    fn publish_event() -> NotificationEvent {
        NotificationEvent::new(
            NotificationEventType::PackagePublished,
            "npm-proxy",
            "left-pad",
            None,
            "alice",
        )
    }

    #[test]
    fn channel_names_returns_configured_channels() {
        let config = webhook_config("wh", "http://example.invalid/hook");
        let svc = make_service(&config);
        assert_eq!(svc.channel_names(), vec!["wh".to_owned()]);
    }

    #[tokio::test]
    async fn subscription_crud_round_trip() {
        let config = webhook_config("wh", "http://example.invalid/hook");
        let svc = make_service(&config);
        let sub = sample_subscription("wh");
        svc.add_subscription(sub.clone()).await.unwrap();

        let listed = svc.list_subscriptions().await.unwrap();
        assert_eq!(listed.len(), 1);

        let fetched = svc.get_subscription(sub.id).await.unwrap().unwrap();
        assert_eq!(fetched.channel_name, "wh");

        let mut updated = fetched.clone();
        updated.enabled = false;
        svc.update_subscription(updated).await.unwrap();
        let fetched = svc.get_subscription(sub.id).await.unwrap().unwrap();
        assert!(!fetched.enabled);

        svc.remove_subscription(sub.id).await.unwrap();
        assert!(svc.get_subscription(sub.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn dispatch_event_with_no_matching_subscription_is_noop() {
        let config = webhook_config("wh", "http://example.invalid/hook");
        let svc = make_service(&config);
        svc.dispatch_event(publish_event()).await;
    }

    #[tokio::test]
    async fn dispatch_event_with_unknown_channel_warns_and_continues() {
        let config = webhook_config("wh", "http://example.invalid/hook");
        let svc = make_service(&config);
        let sub = sample_subscription("does-not-exist");
        svc.add_subscription(sub).await.unwrap();

        svc.dispatch_event(publish_event()).await;
    }

    #[tokio::test]
    async fn dispatch_event_dispatches_to_matching_channel() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/hook")
            .with_status(200)
            .create_async()
            .await;
        let config = webhook_config("wh", &format!("{}/hook", server.url()));
        let svc = make_service(&config);
        svc.add_subscription(sample_subscription("wh"))
            .await
            .unwrap();

        svc.dispatch_event(publish_event()).await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn dispatch_event_background_then_shutdown_completes() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/hook")
            .with_status(200)
            .create_async()
            .await;
        let config = webhook_config("wh", &format!("{}/hook", server.url()));
        let svc = make_service(&config);
        svc.add_subscription(sample_subscription("wh"))
            .await
            .unwrap();

        svc.dispatch_event_background(publish_event());
        svc.shutdown().await;
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_subscription_succeeds_for_configured_channel() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/hook")
            .with_status(200)
            .create_async()
            .await;
        let config = webhook_config("wh", &format!("{}/hook", server.url()));
        let svc = make_service(&config);
        svc.test_subscription(&sample_subscription("wh"))
            .await
            .unwrap();
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_subscription_fails_for_unknown_channel() {
        let config = webhook_config("wh", "http://example.invalid/hook");
        let svc = make_service(&config);
        let err = svc
            .test_subscription(&sample_subscription("missing"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("missing"));
    }
}
