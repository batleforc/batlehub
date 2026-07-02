use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::BatleHubClient;

// ── Quota ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaEntry {
    pub registry: String,
    pub user_id: String,
    #[serde(rename = "bytes_published")]
    pub storage_bytes: u64,
    #[serde(rename = "packages_count")]
    pub package_count: u32,
}

// ── IP blocking ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpBlockEntry {
    pub ip: String,
    pub reason: String,
    pub blocked_at: u64,
    pub unblock_at: u64,
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
    #[serde(rename = "triggered_at")]
    pub applied_at: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
}

// ── Warm ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct WarmRequest {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<String>,
    /// Upstream artifact paths for path-addressed registries (deb/rpm/jetbrains).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

// ── Audit log ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditPackageRef {
    pub registry: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditOutcome {
    pub outcome: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub user_id: Option<String>,
    pub package_id: Option<AuditPackageRef>,
    pub action: Option<String>,
    pub timestamp: Option<String>,
    pub result: Option<AuditOutcome>,
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
pub struct PurgeAuditLogResponse {
    pub deleted: u64,
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStatsEntry {
    pub registry: String,
    pub artifact_hits: u64,
    pub artifact_misses: u64,
    /// Artifact hit rate in [0, 1], or null if no requests yet.
    pub hit_rate: Option<f64>,
    /// Total bytes cached in storage for this registry (from storage backend).
    pub cached_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggregateStats {
    pub artifact_hits: u64,
    pub artifact_misses: u64,
    pub hit_rate: Option<f64>,
    pub cached_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResponse {
    /// When the server process started (counters reset on restart).
    pub since_startup: DateTime<Utc>,
    pub aggregate: AggregateStats,
    pub per_registry: Vec<RegistryStatsEntry>,
}

// ── Health ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAccessInfo {
    pub roles: Vec<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentErrorEntry {
    pub timestamp: DateTime<Utc>,
    pub user_id: Option<String>,
    pub package_name: String,
    pub version: String,
    /// "denied" (blocked / RBAC) or "error" (upstream proxy failure).
    pub error_type: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryHealthEntry {
    pub registry: String,
    pub registry_type: String,
    pub package_count: i64,
    pub cached_artifact_count: i64,
    pub total_size_bytes: Option<i64>,
    pub last_pull_at: Option<DateTime<Utc>>,
    pub pulls_last_hour: i64,
    pub pulls_last_day: i64,
    pub recent_errors: Vec<RecentErrorEntry>,
    pub access: RegistryAccessInfo,
}

// ── Package visibility ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisibilityResponse {
    pub visibility: String,
}

// ── Team namespaces ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamNamespaceEntry {
    pub registry: String,
    pub prefix: String,
    pub group_id: String,
    pub claimed_by: Option<String>,
}

// ── User blocks ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedUserEntry {
    pub user_id: String,
    pub blocked_at: DateTime<Utc>,
    pub blocked_by: String,
    pub reason: Option<String>,
}

// ── Notifications ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationChannelEntry {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)] // mirrors batlehub_core::entities::NotificationEventType
pub enum NotificationEventTypeEntry {
    PackagePublished,
    PackageYanked,
    PackageUnyanked,
    PackageDeleted,
}

impl std::fmt::Display for NotificationEventTypeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::PackagePublished => "package_published",
            Self::PackageYanked => "package_yanked",
            Self::PackageUnyanked => "package_unyanked",
            Self::PackageDeleted => "package_deleted",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSubscriptionEntry {
    pub id: Uuid,
    /// `None` matches all registries.
    pub registry: Option<String>,
    /// `None` matches all packages in the selected registries.
    pub package_name: Option<String>,
    pub event_types: Vec<NotificationEventTypeEntry>,
    pub channel_name: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub enabled: bool,
}

// ── Bulk operations ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkPackageFailureEntry {
    pub name: String,
    pub version: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkPackageResult {
    pub processed: usize,
    pub succeeded: usize,
    pub failed: Vec<BulkPackageFailureEntry>,
}

// ── Config content ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigContentResponse {
    pub content: String,
    /// True when hot reload is disabled (e.g. Kubernetes ConfigMap mount).
    pub is_readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedRegistryEntry {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReloadDiff {
    pub added_registries: Vec<String>,
    pub removed_registries: Vec<String>,
    pub changed_registries: Vec<ChangedRegistryEntry>,
    pub access_config_changed: bool,
    pub limits_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReloadResponse {
    pub diff: ReloadDiff,
}

// ── Access check ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SimulateAccessRequest {
    pub registry: String,
    pub package_name: String,
    pub version: String,
    pub resource_type: String,
    /// Simulated user id (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Simulated role: "anonymous", "user", or "admin". Defaults to "anonymous".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Simulated OIDC groups the identity belongs to.
    #[serde(default)]
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessSimulationResponse {
    /// "allow" or "deny".
    pub decision: String,
    /// Present when decision is "deny".
    pub reason: Option<String>,
    /// Name of the rule that triggered the deny, if any.
    pub rule_matched: Option<String>,
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
        #[derive(Deserialize)]
        struct Wrapper {
            items: Vec<ConfigChangeEntry>,
        }
        let w: Wrapper = self.get("/api/v1/admin/config/changes").await?;
        Ok(w.items)
    }

    // ── Cache ──────────────────────────────────────────────────────────────────

    pub async fn cache_warm(
        &self,
        registry: &str,
        packages: Vec<String>,
        paths: Vec<String>,
    ) -> Result<()> {
        let body = WarmRequest { packages, paths };
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

    pub async fn audit_log(&self, query: AuditQuery) -> Result<Vec<AuditEntry>> {
        self.get_with_params("/api/v1/admin/audit-log", &query)
            .await
    }

    pub async fn purge_audit_log(&self, before: &str) -> Result<PurgeAuditLogResponse> {
        #[derive(serde::Serialize)]
        struct Q<'a> {
            before: &'a str,
        }
        self.delete_with_params_json("/api/v1/admin/audit-log", &Q { before })
            .await
    }

    // ── Stats ──────────────────────────────────────────────────────────────────

    pub async fn admin_stats(&self) -> Result<StatsResponse> {
        self.get("/api/v1/admin/stats").await
    }

    // ── Health ─────────────────────────────────────────────────────────────────

    pub async fn registry_health(&self) -> Result<Vec<RegistryHealthEntry>> {
        self.get("/api/v1/admin/health").await
    }

    // ── Visibility ─────────────────────────────────────────────────────────────

    pub async fn get_visibility(&self, registry: &str, name: &str) -> Result<VisibilityResponse> {
        self.get(&format!(
            "/api/v1/admin/registries/{registry}/packages/{name}/visibility"
        ))
        .await
    }

    pub async fn set_visibility(&self, registry: &str, name: &str, visibility: &str) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            visibility: &'a str,
        }
        self.put(
            &format!("/api/v1/admin/registries/{registry}/packages/{name}/visibility"),
            &Body { visibility },
        )
        .await
    }

    // ── Team namespaces ────────────────────────────────────────────────────────

    pub async fn list_namespaces(&self, registry: &str) -> Result<Vec<TeamNamespaceEntry>> {
        self.get(&format!("/api/v1/admin/registries/{registry}/namespaces"))
            .await
    }

    pub async fn claim_namespace(
        &self,
        registry: &str,
        prefix: &str,
        group_id: &str,
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            prefix: &'a str,
            group_id: &'a str,
        }
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/namespaces"),
            &Body { prefix, group_id },
        )
        .await
    }

    pub async fn release_namespace(&self, registry: &str, prefix: &str) -> Result<()> {
        self.delete(&format!(
            "/api/v1/admin/registries/{registry}/namespaces/{prefix}"
        ))
        .await
    }

    // ── User blocks ────────────────────────────────────────────────────────────

    pub async fn list_blocked_users(&self) -> Result<Vec<BlockedUserEntry>> {
        self.get("/api/v1/admin/users/blocked").await
    }

    pub async fn block_user(&self, user_id: &str, reason: Option<&str>) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            reason: Option<&'a str>,
        }
        self.post_void(
            &format!("/api/v1/admin/users/{user_id}/block"),
            &Body { reason },
        )
        .await
    }

    pub async fn unblock_user(&self, user_id: &str) -> Result<()> {
        self.delete(&format!("/api/v1/admin/users/{user_id}/block"))
            .await
    }

    // ── SBOM ───────────────────────────────────────────────────────────────────
    //
    // The server serializes the raw SBOM document (`s.document` in
    // `crates/web/src/handlers/back_office/sbom.rs`), whose shape is SPDX or
    // CycloneDX JSON depending on the requested `format` — genuinely dynamic,
    // not a fixed server-side struct. `serde_json::Value` is the correct type
    // here rather than a DTO that would misrepresent one arbitrary schema.

    pub async fn get_sbom(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        format: &str,
    ) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Q<'a> {
            format: &'a str,
        }
        self.get_with_params(
            &format!("/api/v1/sbom/{registry}/{name}/{version}"),
            &Q { format },
        )
        .await
    }

    pub async fn export_sbom(
        &self,
        registry: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        format: &str,
    ) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Q<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            registry: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            from: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            to: Option<&'a str>,
            format: &'a str,
        }
        self.get_with_params(
            "/api/v1/sbom/export",
            &Q {
                registry,
                from,
                to,
                format,
            },
        )
        .await
    }

    // ── Notifications ──────────────────────────────────────────────────────────

    pub async fn list_notification_channels(&self) -> Result<Vec<NotificationChannelEntry>> {
        #[derive(Deserialize)]
        struct Wrapper {
            channels: Vec<NotificationChannelEntry>,
        }
        let w: Wrapper = self.get("/api/v1/admin/notifications/channels").await?;
        Ok(w.channels)
    }

    pub async fn list_notification_subscriptions(
        &self,
    ) -> Result<Vec<NotificationSubscriptionEntry>> {
        self.get("/api/v1/admin/notifications/subscriptions").await
    }

    pub async fn delete_notification_subscription(&self, id: &str) -> Result<()> {
        self.delete(&format!("/api/v1/admin/notifications/subscriptions/{id}"))
            .await
    }

    // ── Bulk operations ────────────────────────────────────────────────────────

    pub async fn bulk_yank(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<BulkPackageResult> {
        self.bulk_operation(registry, "bulk-yank", packages).await
    }

    pub async fn bulk_unyank(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<BulkPackageResult> {
        self.bulk_operation(registry, "bulk-unyank", packages).await
    }

    pub async fn bulk_delete(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<BulkPackageResult> {
        self.bulk_operation(registry, "bulk-delete", packages).await
    }

    async fn bulk_operation(
        &self,
        registry: &str,
        op: &str,
        packages: Vec<(String, String)>,
    ) -> Result<BulkPackageResult> {
        #[derive(serde::Serialize)]
        struct PkgRef {
            name: String,
            version: String,
        }
        #[derive(serde::Serialize)]
        struct Body {
            packages: Vec<PkgRef>,
        }
        let body = Body {
            packages: packages
                .into_iter()
                .map(|(name, version)| PkgRef { name, version })
                .collect(),
        };
        self.post(&format!("/api/v1/admin/registries/{registry}/{op}"), &body)
            .await
    }

    // ── Config content ─────────────────────────────────────────────────────────

    pub async fn config_content(&self) -> Result<ConfigContentResponse> {
        self.get("/api/v1/admin/config/content").await
    }

    pub async fn config_validate(&self, content: &str) -> Result<ReloadResponse> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            content: &'a str,
        }
        self.post("/api/v1/admin/config/validate", &Body { content })
            .await
    }

    pub async fn config_from_content(&self, content: &str) -> Result<ReloadResponse> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            content: &'a str,
        }
        self.post("/api/v1/admin/config/from-content", &Body { content })
            .await
    }

    // ── Deprecate / unlist ─────────────────────────────────────────────────────

    pub async fn deprecate_package(
        &self,
        registry: &str,
        name: &str,
        version: &str,
        message: Option<&str>,
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            name: &'a str,
            version: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            message: Option<&'a str>,
        }
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/deprecate"),
            &Body {
                name,
                version,
                message,
            },
        )
        .await
    }

    pub async fn undeprecate_package(
        &self,
        registry: &str,
        name: &str,
        version: &str,
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            name: &'a str,
            version: &'a str,
        }
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/undeprecate"),
            &Body { name, version },
        )
        .await
    }

    pub async fn unlist_package(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            name: &'a str,
            version: &'a str,
        }
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/unlist"),
            &Body { name, version },
        )
        .await
    }

    pub async fn relist_package(&self, registry: &str, name: &str, version: &str) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            name: &'a str,
            version: &'a str,
        }
        self.post_void(
            &format!("/api/v1/admin/registries/{registry}/relist"),
            &Body { name, version },
        )
        .await
    }

    pub async fn simulate_access(
        &self,
        req: &SimulateAccessRequest,
    ) -> Result<AccessSimulationResponse> {
        self.post("/api/v1/admin/access-check", req).await
    }

    pub async fn export_audit_log(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        registry: Option<&str>,
        format: &str,
    ) -> Result<String> {
        use anyhow::bail;
        use reqwest::Method;
        #[derive(serde::Serialize)]
        struct Params<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            from: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            to: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            registry: Option<&'a str>,
            format: &'a str,
        }
        let req = self
            .request(Method::GET, "/api/v1/admin/audit-log/export")
            .query(&Params {
                from,
                to,
                registry,
                format,
            });
        let resp = self.send(req).await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp.text().await?)
        } else {
            let body = resp.text().await.unwrap_or_default();
            bail!("HTTP {status}: {body}")
        }
    }
}
