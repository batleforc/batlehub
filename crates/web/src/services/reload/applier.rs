use chrono::{DateTime, Utc};
use uuid::Uuid;

use batlehub_config::load_from_str as load_config_from_str;
use batlehub_core::entities::{BannerLevel, GlobalBanner};

use super::{ConfigReloadService, PendingReload, ReloadDiff, ReloadSource, PENDING_TTL_SECS};

/// The 3 distinguishable failure modes of [`ConfigReloadService::apply`],
/// wrapped into the `anyhow::Error` it returns so callers that need to tell
/// them apart (e.g. `apply_pending_reload`'s HTTP status mapping) can
/// `downcast_ref` instead of substring-matching the error's `Display` text.
#[derive(Debug, thiserror::Error)]
pub enum ReloadApplyError {
    #[error("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)")]
    Disabled,
    #[error("no pending reload")]
    NoPendingReload,
    #[error("pending reload expired")]
    Expired,
}

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
        let content = tokio::fs::read_to_string(&self.config_path).await?;
        if self.mark_seen_and_check_unchanged(&content) {
            // File-watcher fired (touch/atomic-save rewrite) but the bytes on disk are
            // identical to the last load attempt — nothing to rebuild.
            return Ok(ReloadDiff::default());
        }
        let new_config = load_config_from_str(&content)?;
        self.build_pending(new_config, source).await
    }

    /// Validates a config TOML string without storing a pending reload.
    /// Returns the diff that would result from applying the new config.
    pub async fn validate_content(&self, content: &str) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let new_config = load_config_from_str(content)?;
        let built = (self.builder)(&new_config)?;
        Ok(self.compute_diff(&built.hot, &built.access).await)
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
        if self.mark_seen_and_check_unchanged(content) {
            return Ok(ReloadDiff::default());
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

    /// Records `content` as the last-seen raw config text and reports whether it is
    /// identical to the previous call's content. `false` on the very first call (there
    /// is nothing yet to compare against), so the first load after startup always goes
    /// through `build_pending` — matching the state the caller already has on disk.
    fn mark_seen_and_check_unchanged(&self, content: &str) -> bool {
        let mut last = self
            .last_content
            .lock()
            .expect("last_content lock poisoned");
        let unchanged = last.as_deref() == Some(content);
        *last = Some(content.to_owned());
        unchanged
    }

    /// Internal helper shared by `load_pending` and `load_pending_from_content`.
    async fn build_pending(
        &self,
        new_config: batlehub_config::AppConfig,
        source: ReloadSource,
    ) -> Result<ReloadDiff, anyhow::Error> {
        let built = (self.builder)(&new_config)?;
        let diff = self.compute_diff(&built.hot, &built.access).await;
        let now = Utc::now();
        let pending = PendingReload {
            id: Uuid::new_v4(),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(PENDING_TTL_SECS),
            source,
            diff: diff.clone(),
            content: None, // set by load_pending_from_content when originating from the editor
            new_hot: built.hot,
            new_access: built.access,
            new_registry_map: built.registry_map,
            new_registry_mode_map: built.registry_mode_map,
            new_upstream_map: built.upstream_map,
            new_cargo_index_map: built.cargo_index_map,
            new_repo_signer_map: built.repo_signer_map,
            new_vuln_db_map: built.vuln_db_map,
        };
        *self.pending.lock().expect("pending reload lock poisoned") = Some(pending);
        Ok(diff)
    }

    /// Applies the current pending reload: swaps hot config + access config, persists audit row,
    /// clears the pending state. Returns the diff that was applied.
    pub async fn apply(&self, triggered_by: &str) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            return Err(ReloadApplyError::Disabled.into());
        }
        let pending = self
            .pending
            .lock()
            .expect("pending reload lock poisoned")
            .take()
            .ok_or(ReloadApplyError::NoPendingReload)?;

        if Utc::now() > pending.expires_at {
            return Err(ReloadApplyError::Expired.into());
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

        // Replace HotConfig, AccessConfig, and all registry metadata maps.
        //
        // Each map below is swapped under its own lock, one after another — this is
        // *not* a single atomic transition. A request handled concurrently with this
        // method can observe the new HotConfig but an old registry_map (or any other
        // in-between combination), for the brief window between these blocks. Every
        // map converges to the new config by the time this function returns; the
        // gap is a request-scoped skew, not a lost update. If a handler ever needs
        // cross-map consistency within a single request, snapshot the specific maps
        // it depends on once at the top of the request instead of assuming reload
        // applies them all in one step.
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
        //
        // Each `replace_from` call takes its own lock pair (read `pending`'s map,
        // write this one) — deliberate, matching `ConfigReloadService::pending`'s
        // policy (see its doc comment in `mod.rs`): a poisoned lock here means a
        // reload already panicked mid-swap, so crashing is preferred over serving
        // a torn config.
        self.registry_map.replace_from(&pending.new_registry_map);
        self.registry_mode_map
            .replace_from(&pending.new_registry_mode_map);
        self.upstream_map.replace_from(&pending.new_upstream_map);
        self.cargo_index_map
            .replace_from(&pending.new_cargo_index_map);
        // Swap the deb/rpm repo signing keys so a reload that adds/changes/removes
        // `[registries.repo_signing]` takes effect without a process restart.
        self.repo_signer_map
            .replace_from(&pending.new_repo_signer_map);
        // Swap the Go vuln DB URL map; reuse the existing HTTP client.
        self.vuln_db_map.replace_from(&pending.new_vuln_db_map);

        // Clear the in-progress banner on success.
        if let Some(ref banner) = self.banner {
            let _ = banner
                .clear()
                .await
                .inspect_err(|e| tracing::warn!(error = %e, "failed to clear reload banner"));
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
        let repo = self
            .config_change_repo
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database not configured"))?;
        let records = repo.list(page, per_page).await?;
        Ok(records.into_iter().map(ConfigChangeRow::from).collect())
    }

    /// Total count of `config_changes` rows, ignoring pagination — backs
    /// `ConfigChangesResponse.total`.
    pub async fn count_changes(&self) -> Result<u64, anyhow::Error> {
        let repo = self
            .config_change_repo
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("database not configured"))?;
        Ok(repo.count().await?)
    }
}

impl From<batlehub_core::ports::ConfigChangeRecord> for ConfigChangeRow {
    fn from(r: batlehub_core::ports::ConfigChangeRecord) -> Self {
        ConfigChangeRow {
            id: r.id,
            triggered_by: r.triggered_by,
            triggered_at: r.triggered_at,
            status: r.status,
            diff: r.diff,
            summary: r.summary,
            error_msg: r.error_msg,
        }
    }
}
