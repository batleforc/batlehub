use serde::Deserialize;

use super::registry::default_true;

// ── Notifications & Webhooks ──────────────────────────────────────────────────

/// Top-level notification configuration block.
///
/// ```toml
/// [notifications]
/// enabled = true
///
/// [[notifications.channels]]
/// name = "my-slack"
/// type = "slack"
/// url = "https://hooks.slack.com/services/..."
///
/// [[notifications.channels]]
/// name = "ci-webhook"
/// type = "webhook"
/// url = "https://example.com/hook"
/// secret = "hmac-secret"
///
/// [[notifications.inbound]]
/// name = "ci-scanner"
/// secret = "verify-me"
/// ```
#[derive(Debug, Deserialize, Default, Clone)]
pub struct NotificationsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub channels: Vec<NotificationChannelConfig>,
    #[serde(default)]
    pub inbound: Vec<InboundWebhookConfig>,
}

/// A single outbound notification channel.
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationChannelConfig {
    Webhook(WebhookChannelConfig),
    Slack(SlackChannelConfig),
    Teams(TeamsChannelConfig),
    Email(EmailChannelConfig),
}

impl NotificationChannelConfig {
    pub fn name(&self) -> &str {
        match self {
            Self::Webhook(c) => &c.name,
            Self::Slack(c) => &c.name,
            Self::Teams(c) => &c.name,
            Self::Email(c) => &c.name,
        }
    }
}

/// Generic outbound HTTP webhook channel.
#[derive(Debug, Deserialize, Clone)]
pub struct WebhookChannelConfig {
    pub name: String,
    pub url: String,
    /// Optional HMAC-SHA256 signing secret. When set, a
    /// `X-BatleHub-Signature-256: sha256=<hex>` header is added to each POST.
    pub secret: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Slack Incoming Webhook channel.
#[derive(Debug, Deserialize, Clone)]
pub struct SlackChannelConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// Microsoft Teams Incoming Webhook channel.
#[derive(Debug, Deserialize, Clone)]
pub struct TeamsChannelConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

/// SMTP email channel.
#[derive(Debug, Deserialize, Clone)]
pub struct EmailChannelConfig {
    pub name: String,
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    pub smtp_user: Option<String>,
    pub smtp_password: Option<String>,
    pub from: String,
    pub to: Vec<String>,
    #[serde(default = "default_true")]
    pub tls: bool,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_timeout_secs() -> u64 {
    10
}

fn default_smtp_port() -> u16 {
    587
}

/// Configuration for a single inbound webhook endpoint.
#[derive(Debug, Deserialize, Clone)]
pub struct InboundWebhookConfig {
    pub name: String,
    /// Optional HMAC-SHA256 secret used to verify the `X-Hub-Signature-256` header.
    /// When absent, any payload is accepted (suitable only for internal networks).
    pub secret: Option<String>,
}
