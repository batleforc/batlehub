use std::sync::Arc;
use std::time::Duration;

use batlehub_config::schema::{
    EmailChannelConfig, SlackChannelConfig, TeamsChannelConfig, WebhookChannelConfig,
};
use batlehub_core::entities::{NotificationEvent, NotificationEventType};
use lettre::{
    message::header::ContentType, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};

// ── Dispatcher trait ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub(super) trait ChannelDispatcher: Send + Sync {
    async fn dispatch(&self, event: &NotificationEvent) -> anyhow::Result<()>;
    /// Send a synthetic test event to verify the channel is reachable.
    async fn send_test(&self) -> anyhow::Result<()>;
}

// ── Webhook dispatcher ────────────────────────────────────────────────────────

pub(super) struct WebhookDispatcher {
    pub url: String,
    pub secret: Option<String>,
    pub client: Arc<reqwest::Client>,
    pub timeout: Duration,
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

pub(super) struct SlackDispatcher {
    pub url: String,
    pub client: Arc<reqwest::Client>,
    pub timeout: Duration,
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

pub(super) struct TeamsDispatcher {
    pub url: String,
    pub client: Arc<reqwest::Client>,
    pub timeout: Duration,
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

pub(super) struct EmailDispatcher {
    pub transport: AsyncSmtpTransport<Tokio1Executor>,
    pub from: String,
    pub to: Vec<String>,
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

// ── Builder helpers ───────────────────────────────────────────────────────────

pub(super) fn build_webhook(
    cfg: &WebhookChannelConfig,
    client: &Arc<reqwest::Client>,
) -> WebhookDispatcher {
    WebhookDispatcher {
        url: cfg.url.clone(),
        secret: cfg.secret.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

pub(super) fn build_slack(
    cfg: &SlackChannelConfig,
    client: &Arc<reqwest::Client>,
) -> SlackDispatcher {
    SlackDispatcher {
        url: cfg.url.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

pub(super) fn build_teams(
    cfg: &TeamsChannelConfig,
    client: &Arc<reqwest::Client>,
) -> TeamsDispatcher {
    TeamsDispatcher {
        url: cfg.url.clone(),
        client: Arc::clone(client),
        timeout: Duration::from_secs(cfg.timeout_secs),
    }
}

pub(super) fn build_email(cfg: &EmailChannelConfig) -> anyhow::Result<EmailDispatcher> {
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

pub(super) fn format_event_text(event: &NotificationEvent) -> String {
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

pub(super) fn test_event() -> NotificationEvent {
    NotificationEvent::new(
        NotificationEventType::PackagePublished,
        "test-registry",
        "test-package",
        Some("0.0.0".to_owned()),
        "batlehub-test",
    )
}

pub(super) fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    hex::encode(mac.finalize().into_bytes())
}
