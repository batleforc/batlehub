use anyhow::Result;
use serde::{Deserialize, Serialize};

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

    pub async fn purge_audit_log(&self, before: &str) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Q<'a> {
            before: &'a str,
        }
        self.delete_with_params_json("/api/v1/admin/audit-log", &Q { before })
            .await
    }

    // ── Stats ──────────────────────────────────────────────────────────────────

    pub async fn admin_stats(&self) -> Result<serde_json::Value> {
        self.get("/api/v1/admin/stats").await
    }

    // ── Health ─────────────────────────────────────────────────────────────────

    pub async fn registry_health(&self) -> Result<serde_json::Value> {
        self.get("/api/v1/admin/health").await
    }

    // ── Visibility ─────────────────────────────────────────────────────────────

    pub async fn get_visibility(&self, registry: &str, name: &str) -> Result<serde_json::Value> {
        self.get(&format!(
            "/api/v1/admin/registries/{registry}/packages/{name}/visibility"
        ))
        .await
    }

    pub async fn set_visibility(
        &self,
        registry: &str,
        name: &str,
        visibility: &str,
    ) -> Result<()> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            visibility: &'a str,
        }
        self.put(
            &format!(
                "/api/v1/admin/registries/{registry}/packages/{name}/visibility"
            ),
            &Body { visibility },
        )
        .await
    }

    // ── Team namespaces ────────────────────────────────────────────────────────

    pub async fn list_namespaces(&self, registry: &str) -> Result<serde_json::Value> {
        self.get(&format!(
            "/api/v1/admin/registries/{registry}/namespaces"
        ))
        .await
    }

    pub async fn claim_namespace(
        &self,
        registry: &str,
        prefix: &str,
        group_id: &str,
    ) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            prefix: &'a str,
            group_id: &'a str,
        }
        self.post(
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

    pub async fn list_blocked_users(&self) -> Result<serde_json::Value> {
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
        self.get_with_params("/api/v1/sbom/export", &Q { registry, from, to, format })
            .await
    }

    // ── Notifications ──────────────────────────────────────────────────────────

    pub async fn list_notification_channels(&self) -> Result<serde_json::Value> {
        self.get("/api/v1/admin/notifications/channels").await
    }

    pub async fn list_notification_subscriptions(&self) -> Result<serde_json::Value> {
        self.get("/api/v1/admin/notifications/subscriptions").await
    }

    pub async fn delete_notification_subscription(&self, id: &str) -> Result<()> {
        self.delete(&format!(
            "/api/v1/admin/notifications/subscriptions/{id}"
        ))
        .await
    }

    // ── Bulk operations ────────────────────────────────────────────────────────

    pub async fn bulk_yank(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<serde_json::Value> {
        self.bulk_operation(registry, "bulk-yank", packages).await
    }

    pub async fn bulk_unyank(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<serde_json::Value> {
        self.bulk_operation(registry, "bulk-unyank", packages).await
    }

    pub async fn bulk_delete(
        &self,
        registry: &str,
        packages: Vec<(String, String)>,
    ) -> Result<serde_json::Value> {
        self.bulk_operation(registry, "bulk-delete", packages).await
    }

    async fn bulk_operation(
        &self,
        registry: &str,
        op: &str,
        packages: Vec<(String, String)>,
    ) -> Result<serde_json::Value> {
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

    pub async fn config_content(&self) -> Result<serde_json::Value> {
        self.get("/api/v1/admin/config/content").await
    }

    pub async fn config_validate(&self, content: &str) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            content: &'a str,
        }
        self.post("/api/v1/admin/config/validate", &Body { content })
            .await
    }

    pub async fn config_from_content(&self, content: &str) -> Result<serde_json::Value> {
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
            &Body { name, version, message },
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

    #[allow(clippy::too_many_arguments)]
    pub async fn simulate_access(
        &self,
        registry: &str,
        package_name: &str,
        version: &str,
        resource_type: &str,
        user_id: Option<&str>,
        role: Option<&str>,
        groups: &[String],
    ) -> Result<serde_json::Value> {
        #[derive(serde::Serialize)]
        struct Body<'a> {
            registry: &'a str,
            package_name: &'a str,
            version: &'a str,
            resource_type: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            user_id: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            role: Option<&'a str>,
            groups: &'a [String],
        }
        self.post(
            "/api/v1/admin/access-check",
            &Body {
                registry,
                package_name,
                version,
                resource_type,
                user_id,
                role,
                groups,
            },
        )
        .await
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
        let mut req = self
            .inner
            .request(Method::GET, self.url("/api/v1/admin/audit-log/export"))
            .query(&Params { from, to, registry, format });
        if let Some(auth) = self.auth_header() {
            req = req.header("Authorization", auth);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp.text().await?)
        } else {
            let body = resp.text().await.unwrap_or_default();
            bail!("HTTP {status}: {body}")
        }
    }
}
