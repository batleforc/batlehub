use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BannerLevel {
    Info,
    Warning,
    Error,
}

/// A global administrator broadcast message shown to all website visitors.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GlobalBanner {
    pub message: String,
    pub level: BannerLevel,
    pub set_at: DateTime<Utc>,
    /// user_id of the admin who set the banner, or `"system"` for auto-set messages.
    pub set_by: String,
}
