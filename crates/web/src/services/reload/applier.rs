use chrono::{DateTime, Utc};
use uuid::Uuid;

use batlehub_config::{load as load_config, load_from_str as load_config_from_str};
use batlehub_core::entities::{BannerLevel, GlobalBanner};

use super::{ConfigReloadService, PendingReload, ReloadDiff, ReloadSource, PENDING_TTL_SECS};

/// A row from the `config_changes` audit table.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ConfigChangeRow {
    pub id: Uuid,
    pub triggered_by: String,
    pub triggered_at: DateTime<Utc>,
    pub status: String,
    pub diff: serde_json::Value,
    pub summary: String,
    pub error_msg: Option<String>,
}

impl ConfigReloadService {
    /// Re-reads the config file, validates, and builds a new HotConfig + AccessConfig.
    /// Stores the result as a pending reload (does NOT apply).
    /// Replaces any existing pending reload.
    pub async fn load_pending(&self, source: ReloadSource) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let new_config = load_config(&self.config_path)?;
        self.build_pending(new_config, source).await
    }

    /// Validates a config TOML string without storing a pending reload.
    /// Returns the diff that would result from applying the new config.
    pub async fn validate_content(&self, content: &str) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let new_config = load_config_from_str(content)?;
        let (new_hot, new_access, ..) = (self.builder)(&new_config)?;
        Ok(self.compute_diff(&new_hot, &new_access).await)
    }

    /// Same as `load_pending` but parses the config from a supplied TOML string
    /// instead of re-reading the file. Used by the config-editor endpoint.
    pub async fn load_pending_from_content(
        &self,
        content: &str,
        source: ReloadSource,
    ) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let new_config = load_config_from_str(content)?;
        let diff = self.build_pending(new_config, source).await?;
        // Store the raw content so apply() can persist it to disk.
        if let Some(ref mut p) = *self.pending.lock().expect("pending reload lock poisoned") {
            p.content = Some(content.to_owned());
        }
        Ok(diff)
    }

    /// Read the current on-disk config content without parsing or validating it.
    /// Non-blocking: uses `tokio::fs` so it does not stall a tokio worker thread.
    pub async fn config_content(&self) -> Result<String, std::io::Error> {
        tokio::fs::read_to_string(&self.config_path).await
    }

    /// Internal helper shared by `load_pending` and `load_pending_from_content`.
    async fn build_pending(
        &self,
        new_config: batlehub_config::AppConfig,
        source: ReloadSource,
    ) -> Result<ReloadDiff, anyhow::Error> {
        let (
            new_hot,
            new_access,
            new_registry_map,
            new_registry_mode_map,
            new_upstream_map,
            new_cargo_index_map,
            new_repo_signer_map,
        ) = (self.builder)(&new_config)?;
        let diff = self.compute_diff(&new_hot, &new_access).await;
        let now = Utc::now();
        let pending = PendingReload {
            id: Uuid::new_v4(),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(PENDING_TTL_SECS),
            source,
            diff: diff.clone(),
            content: None, // set by load_pending_from_content when originating from the editor
            new_hot,
            new_access,
            new_registry_map,
            new_registry_mode_map,
            new_upstream_map,
            new_cargo_index_map,
            new_repo_signer_map,
        };
        *self.pending.lock().expect("pending reload lock poisoned") = Some(pending);
        Ok(diff)
    }

    /// Applies the current pending reload: swaps hot config + access config, persists audit row,
    /// clears the pending state. Returns the diff that was applied.
    pub async fn apply(&self, triggered_by: &str) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let pending = self
            .pending
            .lock()
            .expect("pending reload lock poisoned")
            .take()
            .ok_or_else(|| anyhow::anyhow!("no pending reload"))?;

        if Utc::now() > pending.expires_at {
            anyhow::bail!("pending reload expired");
        }

        // Set "reload in progress" banner while swapping.
        if let Some(ref banner) = self.banner {
            let _ = banner
                .set(GlobalBanner {
                    message: "Configuration reload in progress…".to_owned(),
                    level: BannerLevel::Info,
                    set_at: Utc::now(),
                    set_by: "system".to_owned(),
                })
                .await;
        }

        // Atomic swap: replace HotConfig, AccessConfig, and all registry metadata maps.
        {
            let mut hot = self.hot.write().await;
            *hot = pending.new_hot;
        }
        {
            let mut access = self.access.write().await;
            *access = pending.new_access;
        }
        // Swap the shared registry/mode/upstream maps in-place so all actix workers
        // immediately see the new registries without a process restart.
        {
            let mut rm = self
                .registry_map
                .0
                .write()
                .expect("registry map lock poisoned");
            *rm = pending
                .new_registry_map
                .0
                .read()
                .expect("registry map lock poisoned")
                .clone();
        }
        {
            let mut mm = self
                .registry_mode_map
                .0
                .write()
                .expect("registry mode map lock poisoned");
            *mm = pending
                .new_registry_mode_map
                .0
                .read()
                .expect("registry mode map lock poisoned")
                .clone();
        }
        {
            let mut um = self
                .upstream_map
                .0
                .write()
                .expect("upstream map lock poisoned");
            *um = pending
                .new_upstream_map
                .0
                .read()
                .expect("upstream map lock poisoned")
                .clone();
        }
        {
            let mut cm = self
                .cargo_index_map
                .0
                .write()
                .expect("cargo index map lock poisoned");
            *cm = pending
                .new_cargo_index_map
                .0
                .read()
                .expect("cargo index map lock poisoned")
                .clone();
        }
        // Swap the deb/rpm repo signing keys so a reload that adds/changes/removes
        // `[registries.repo_signing]` takes effect without a process restart.
        {
            let mut sm = self
                .repo_signer_map
                .0
                .write()
                .expect("repo signer map lock poisoned");
            *sm = pending
                .new_repo_signer_map
                .0
                .read()
                .expect("repo signer map lock poisoned")
                .clone();
        }

        // Clear the in-progress banner on success.
        if let Some(ref banner) = self.banner {
            let _ = banner.clear().await;
        }

        // Write editor-submitted content back to disk so the change survives a restart.
        // File-watcher reloads set content = None (the file is already correct on disk).
        if let Some(ref text) = pending.content {
            if let Err(e) = tokio::fs::write(&self.config_path, text).await {
                tracing::warn!(
                    path = %self.config_path,
                    error = %e,
                    "failed to persist editor config to disk; change is live in memory but will be lost on restart"
                );
            }
        }

        // Persist audit row (best-effort — do not fail the reload if DB is unavailable).
        self.persist_audit(&pending.diff, triggered_by, "applied", None)
            .await;

        Ok(pending.diff)
    }

    /// Returns the history of applied reloads from `config_changes`.
    pub async fn list_changes(
        &self,
        page: u64,
        per_page: u64,
    ) -> Result<Vec<ConfigChangeRow>, anyhow::Error> {
        let pool = self
            .pool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database not configured"))?;
        let offset = (page * per_page) as i64;
        let limit = per_page as i64;
        let rows = sqlx::query(
            "SELECT id, triggered_by, triggered_at, status, diff, summary, error_msg
             FROM config_changes
             ORDER BY triggered_at DESC
             LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|r| {
            use sqlx::Row;
            ConfigChangeRow {
                id: r.get("id"),
                triggered_by: r.get("triggered_by"),
                triggered_at: r.get("triggered_at"),
                status: r.get("status"),
                diff: r
                    .try_get::<serde_json::Value, _>("diff")
                    .unwrap_or(serde_json::Value::Object(Default::default())),
                summary: r.get("summary"),
                error_msg: r.try_get("error_msg").ok(),
            }
        })
        .collect();
        Ok(rows)
    }
}
