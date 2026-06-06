use std::{collections::HashMap, sync::Arc, time::Duration};

const MAX_CONCURRENT_DISPATCHES: usize = 64;

use batlehub_config::schema::{
    EmailChannelConfig, NotificationChannelConfig, NotificationsConfig, SlackChannelConfig,
    TeamsChannelConfig, WebhookChannelConfig,
};
use batlehub_core::{
    entities::{NotificationEvent, NotificationEventType, NotificationSubscription},
    ports::NotificationPort,
};
use hmac::{Hmac, KeyInit, Mac};
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use sha2::Sha256;
use uuid::Uuid;

// ── Dispatcher trait ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
trait ChannelDispatcher: Send + Sync {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()>;
    /// Send a synthetic test event to verify the channel is reachable.
    async fn send_test(&self) -> anyhow::Result<()>;
}

// ── Webhook dispatcher ────────────────────────────────────────────────────────

struct WebhookDispatcher {
    url: String,
    secret: Option<String>,
    client: Arc<reqwest::Client>,
    timeout: Duration,
}

#[async_trait::async_trait]
impl ChannelDispatcher for WebhookDispatcher {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()> {
        let body = serde_json::to_string(event)?;
        let mut req = self
            .client
            .post(&self.url)
            .timeout(self.timeout)
            .header("Content-Type", "application/json")
            .header("X-BatleHub-Event", event.event_type.as_str());

        if let Some(secret) = &self.secret {
            let sig = hmac_sha256_hex(secret.as_bytes(), body.as_bytes());
            req = req.header("X-BatleHub-Signature-256", format!("sha256={sig}"));
        }

        let resp = req.body(body).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("webhook returned HTTP {}", resp.status());
        }
        Ok(())
    }

    async fn send_test(&self) -> anyhow::Result<()> {
        let event = test_event();
        self.dispatch(&event).await
    }
}

// ── Slack dispatcher ──────────────────────────────────────────────────────────

struct SlackDispatcher {
    url: String,
    client: Arc<reqwest::Client>,
    timeout: Duration,
}

#[async_trait::async_trait]
impl ChannelDispatcher for SlackDispatcher {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()> {
        let text = format_event_text(event);
        let payload = serde_json::json!({
            "text": text,
            "blocks": [{
                "type": "section",
                "text": { "type": "mrkdwn", "text": text }
            }]
        });
        let resp = self
            .client
            .post(&self.url)
            .timeout(self.timeout)
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("slack returned HTTP {}", resp.status());
        }
        Ok(())
    }

    async fn send_test(&self) -> anyhow::Result<()> {
        self.dispatch(&test_event()).await
    }
}

// ── Teams dispatcher ──────────────────────────────────────────────────────────

struct TeamsDispatcher {
    url: String,
    client: Arc<reqwest::Client>,
    timeout: Duration,
}

#[async_trait::async_trait]
impl ChannelDispatcher for TeamsDispatcher {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()> {
        let text = format_event_text(event);
        // Teams Adaptive Card (simple MessageCard for compatibility)
        let payload = serde_json::json!({
            "@type": "MessageCard",
            "@context": "https://schema.org/extensions",
            "themeColor": "0076D7",
            "summary": text,
            "sections": [{
                "activityTitle": "BatleHub Notification",
                "activityText": text
            }]
        });
        let resp = self
            .client
            .post(&self.url)
            .timeout(self.timeout)
            .json(&payload)
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("teams returned HTTP {}", resp.status());
        }
        Ok(())
    }

    async fn send_test(&self) -> anyhow::Result<()> {
        self.dispatch(&test_event()).await
    }
}

// ── Email dispatcher ──────────────────────────────────────────────────────────

struct EmailDispatcher {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: String,
    to: Vec<String>,
}

#[async_trait::async_trait]
impl ChannelDispatcher for EmailDispatcher {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()> {
        let subject = format!(
            "[BatleHub] {} — {}/{}",
            event.event_type, event.registry, event.package_name
        );
        let body = format_event_text(event);

        let from_addr: lettre::message::Mailbox = self.from.parse()?;
        let mut errors: Vec<String> = Vec::new();
        for recipient in &self.to {
            let to_addr = match recipient.parse() {
                Ok(a) => a,
                Err(e) => {
                    tracing::warn!(recipient, "email: invalid recipient address: {e}");
                    errors.push(format!("{recipient}: {e}"));
                    continue;
                }
            };
            let email = Message::builder()
                .from(from_addr.clone())
                .to(to_addr)
                .subject(&subject)
                .header(ContentType::TEXT_PLAIN)
                .body(body.clone())?;
            if let Err(e) = self.transport.send(email).await {
                tracing::warn!(recipient, "email: send failed: {e}");
                errors.push(format!("{recipient}: {e}"));
                // Break on the first transport failure — remaining sends would also
                // hang until timeout (e.g. SMTP server down), exhausting semaphore slots.
                break;
            }
        }
        if !errors.is_empty() {
            anyhow::bail!(
                "email delivery failed for {} recipient(s): {}",
                errors.len(),
                errors.join("; ")
            );
        }
        Ok(())
    }

    async fn send_test(&self) -> anyhow::Result<()> {
        self.dispatch(&test_event()).await
    }
}

// ── NotificationService ───────────────────────────────────────────────────────

/// Evaluates subscriptions and dispatches outbound notifications.
///
/// Channels are built once from `NotificationsConfig` at startup.
/// A shared `reqwest::Client` is used for all HTTP-based channels.
pub struct NotificationService {
    store: Arc<dyn NotificationPort>,
    channels: HashMap<String, Box<dyn ChannelDispatcher>>,
    /// Tracks in-flight background dispatch tasks for graceful shutdown.
    pending: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Caps the number of concurrently-running dispatch tasks to prevent burst storms.
    dispatch_semaphore: Arc<tokio::sync::Semaphore>,
}

impl NotificationService {
    pub fn new(store: Arc<dyn NotificationPort>, config: &NotificationsConfig) -> Self {
        let client = Arc::new(
            reqwest::Client::builder()
                .user_agent("batlehub-notifications/0.1")
                .build()
                .expect("notification HTTP client"),
        );

        let mut channels: HashMap<String, Box<dyn ChannelDispatcher>> = HashMap::new();
        for ch in &config.channels {
            let name = ch.name().to_owned();
            let dispatcher: Box<dyn ChannelDispatcher> = match ch {
                NotificationChannelConfig::Webhook(cfg) => Box::new(build_webhook(cfg, &client)),
                NotificationChannelConfig::Slack(cfg) => Box::new(build_slack(cfg, &client)),
                NotificationChannelConfig::Teams(cfg) => Box::new(build_teams(cfg, &client)),
                NotificationChannelConfig::Email(cfg) => match build_email(cfg) {
                    Ok(d) => Box::new(d),
                    Err(e) => {
                        tracing::error!(channel = %name, "notification: failed to build email dispatcher: {e}");
                        continue;
                    }
                },
            };
            channels.insert(name, dispatcher);
        }

        Self {
            store,
            channels,
            pending: std::sync::Mutex::new(Vec::new()),
            dispatch_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_DISPATCHES)),
        }
    }

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
                    "notification: dispatch semaphore full ({MAX_CONCURRENT_DISPATCHES} in-flight), dropping event"
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

    pub fn store(&self) -> &Arc<dyn NotificationPort> {
        &self.store
    }
}

// ── Builder helpers ───────────────────────────────────────────────────────────

fn build_webhook(cfg: &WebhookChannelConfig, client: &Arc<reqwest::Client>) -> WebhookDispatcher {
    WebhookDispatcher {
        url: cfg.url.clone(),
        secret: cfg.secret.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

fn build_slack(cfg: &SlackChannelConfig, client: &Arc<reqwest::Client>) -> SlackDispatcher {
    SlackDispatcher {
        url: cfg.url.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

fn build_teams(cfg: &TeamsChannelConfig, client: &Arc<reqwest::Client>) -> TeamsDispatcher {
    TeamsDispatcher {
        url: cfg.url.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

fn build_email(cfg: &EmailChannelConfig) -> anyhow::Result<EmailDispatcher> {
    // tls = true (default) → STARTTLS (port 587); tls = false → plain SMTP
    let transport = if cfg.tls {
        let builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.smtp_host)?
            .port(cfg.smtp_port);
        match (&cfg.smtp_user, &cfg.smtp_password) {
            (Some(u), Some(p)) => builder.credentials(Credentials::new(u.clone(), p.clone())),
            _ => builder,
        }
        .build()
    } else {
        let builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.smtp_host)
            .port(cfg.smtp_port);
        match (&cfg.smtp_user, &cfg.smtp_password) {
            (Some(u), Some(p)) => builder.credentials(Credentials::new(u.clone(), p.clone())),
            _ => builder,
        }
        .build()
    };
    Ok(EmailDispatcher {
        transport,
        from: cfg.from.clone(),
        to: cfg.to.clone(),
    })
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn format_event_text(event: &NotificationEvent) -> String {
    let version_part = event
        .version
        .as_deref()
        .map(|v| format!(" v{v}"))
        .unwrap_or_default();
    format!(
        "[{}] {}/{}{version_part} — actor: {}",
        event.event_type, event.registry, event.package_name, event.actor
    )
}

fn test_event() -> NotificationEvent {
    NotificationEvent::new(
        NotificationEventType::PackagePublished,
        "test-registry",
        "test-package",
        Some("0.0.0".to_owned()),
        "batlehub-test",
    )
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}

// ── Inbound webhook HMAC verification ────────────────────────────────────────

/// Verify a `X-Hub-Signature-256: sha256=<hex>` header against the request body.
pub fn verify_inbound_hmac(secret: &str, body: &[u8], header_value: &str) -> bool {
    let expected = format!("sha256={}", hmac_sha256_hex(secret.as_bytes(), body));
    // Constant-time compare via hmac verify_slice equivalent (manual XOR).
    if expected.len() != header_value.len() {
        return false;
    }
    expected
        .bytes()
        .zip(header_value.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

// ── Subscription store passthrough (used by handlers) ────────────────────────

impl NotificationService {
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
