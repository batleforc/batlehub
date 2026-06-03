use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::BatleHubClient;

// ── Quota ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaEntry {
    pub registry: String,
    pub user_id: String,
    pub storage_bytes: i64,
    pub package_count: i64,
}

// ── IP blocking ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBlockEntry {
    pub ip: String,
    pub reason: Option<String>,
    pub blocked_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AddIpBlockRequest {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ── Banner ────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SetBannerRequest {
    pub message: String,
    pub level: String,
}

// ── Config ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChangeEntry {
    pub id: Option<String>,
    pub triggered_by: Option<String>,
    pub source: Option<String>,
    pub applied_at: Option<String>,
    pub summary: Option<String>,
}

// ── Warm ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WarmRequest {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<String>,
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub registry: Option<String>,
    pub user_id: Option<String>,
    pub action: Option<String>,
    pub package: Option<String>,
    pub timestamp: Option<String>,
    pub denied: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AuditQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denied_only: Option<bool>,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogResponse {
    pub items: Vec<AuditEntry>,
    pub total: usize,
}

impl BatleHubClient {
    // ── Quota ──────────────────────────────────────────────────────────────────

    pub async fn list_quota(&self, registry: Option<&str>) -> Result<Vec<QuotaEntry>> {
        match registry {
            Some(r) => self.get(&format!("/api/v1/admin/quota/{r}")).await,
            None => self.get("/api/v1/admin/quota").await,
        }
    }

    pub async fn reset_quota(&self, registry: &str, user_id: &str) -> Result<()> {
        self.delete(&format!("/api/v1/admin/quota/{registry}/{user_id}"))
            .await
    }

    // ── IP blocking ────────────────────────────────────────────────────────────

    pub async fn list_ip_blocks(&self) -> Result<Vec<IpBlockEntry>> {
        self.get("/api/v1/admin/ip-blocks").await
    }

    pub async fn add_ip_block(&self, ip: &str, reason: Option<&str>) -> Result<()> {
        let body = AddIpBlockRequest {
            ip: ip.to_string(),
            reason: reason.map(str::to_string),
        };
        self.post_void("/api/v1/admin/ip-blocks", &body).await
    }

    pub async fn remove_ip_block(&self, ip: &str) -> Result<()> {
        self.delete(&format!("/api/v1/admin/ip-blocks/{ip}")).await
    }

    // ── Config ─────────────────────────────────────────────────────────────────

    pub async fn config_reload(&self) -> Result<()> {
        self.post_no_body("/api/v1/admin/config/reload").await
    }

    pub async fn config_changes(&self) -> Result<Vec<ConfigChangeEntry>> {
        self.get("/api/v1/admin/config/changes").await
    }

    // ── Cache ──────────────────────────────────────────────────────────────────

    pub async fn cache_warm(&self, registry: &str, packages: Vec<String>) -> Result<()> {
        let body = WarmRequest { packages };
        self.post_void(&format!("/api/v1/admin/registries/{registry}/warm"), &body)
            .await
    }

    pub async fn cache_clear(&self, registry: &str) -> Result<()> {
        self.post_no_body(&format!("/api/v1/admin/registries/{registry}/clear-cache"))
            .await
    }

    // ── Banner ─────────────────────────────────────────────────────────────────

    pub async fn set_banner(&self, message: &str, level: &str) -> Result<()> {
        let body = SetBannerRequest {
            message: message.to_string(),
            level: level.to_string(),
        };
        self.put("/api/v1/admin/banner", &body).await
    }

    pub async fn clear_banner(&self) -> Result<()> {
        self.delete("/api/v1/admin/banner").await
    }

    // ── Audit log ──────────────────────────────────────────────────────────────

    pub async fn audit_log(&self, query: AuditQuery) -> Result<AuditLogResponse> {
        self.get_with_params("/api/v1/admin/audit-log", &query)
            .await
    }
}
