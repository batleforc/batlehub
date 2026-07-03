use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use batlehub_core::ports::ConfigChangeRepository;
use batlehub_core::services::{HotConfig, HotConfigLock};

use crate::{
    services::banner::BannerService, AccessConfig, AccessConfigLock, CargoIndexMap, RegistryMap,
    RegistryModeMap, RepoSignerMap, UpstreamMap, VulnDbMap,
};

pub(super) const PENDING_TTL_SECS: i64 = 600; // 10 minutes

pub mod applier;
pub mod validator;

pub use applier::ConfigChangeRow;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ChangedRegistry {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ReloadDiff {
    pub added_registries: Vec<String>,
    pub removed_registries: Vec<String>,
    pub changed_registries: Vec<ChangedRegistry>,
    pub access_config_changed: bool,
    pub limits_changed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadSource {
    FileWatcher,
    AdminRequest,
}

pub struct PendingReload {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub source: ReloadSource,
    pub diff: ReloadDiff,
    /// Raw TOML submitted via the config-editor API. Present only when the pending
    /// reload originated from `load_pending_from_content`; absent for file-watcher
    /// reloads (the file is already on disk). `apply()` writes this back to disk so
    /// editor changes survive a server restart.
    pub content: Option<String>,
    /// Pre-built, ready to swap in — no async work required during apply.
    pub new_hot: HotConfig,
    pub new_access: AccessConfig,
    pub new_registry_map: RegistryMap,
    pub new_registry_mode_map: RegistryModeMap,
    pub new_upstream_map: UpstreamMap,
    pub new_cargo_index_map: CargoIndexMap,
    pub new_repo_signer_map: RepoSignerMap,
    pub new_vuln_db_map: VulnDbMap,
}

/// Snapshot of a pending reload returned to the GET /pending endpoint.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PendingReloadSnapshot {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub source: ReloadSource,
    pub diff: ReloadDiff,
}

/// Builds a new hot-reloadable bundle from an `AppConfig`.
///
/// Created in `server/src/main.rs` as a closure capturing `beta_channel_store`,
/// `repo`, etc. Passed to `ConfigReloadService` so the service can rebuild state
/// on reload without depending on adapter types directly.
pub type HotConfigBuilder = Arc<
    dyn Fn(
            &batlehub_config::schema::AppConfig,
        ) -> anyhow::Result<(
            HotConfig,
            AccessConfig,
            RegistryMap,
            RegistryModeMap,
            UpstreamMap,
            CargoIndexMap,
            RepoSignerMap,
            VulnDbMap,
        )> + Send
        + Sync,
>;

/// Service responsible for hot-reloading configuration at runtime.
pub struct ConfigReloadService {
    pub(super) hot: HotConfigLock,
    pub(super) access: AccessConfigLock,
    pub(super) registry_map: RegistryMap,
    pub(super) registry_mode_map: RegistryModeMap,
    pub(super) upstream_map: UpstreamMap,
    pub(super) cargo_index_map: CargoIndexMap,
    pub(super) repo_signer_map: RepoSignerMap,
    pub(super) vuln_db_map: VulnDbMap,
    pub(super) config_path: String,
    pub(super) config_change_repo: Option<Arc<dyn ConfigChangeRepository>>,
    pub hot_reload_enabled: bool,
    pub(super) pending: Mutex<Option<PendingReload>>,
    pub(super) builder: HotConfigBuilder,
    pub(super) banner: Option<Arc<BannerService>>,
}

impl ConfigReloadService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hot: HotConfigLock,
        access: AccessConfigLock,
        registry_map: RegistryMap,
        registry_mode_map: RegistryModeMap,
        upstream_map: UpstreamMap,
        cargo_index_map: CargoIndexMap,
        repo_signer_map: RepoSignerMap,
        vuln_db_map: VulnDbMap,
        config_path: String,
        config_change_repo: Option<Arc<dyn ConfigChangeRepository>>,
        hot_reload_enabled: bool,
        builder: HotConfigBuilder,
        banner: Option<Arc<BannerService>>,
    ) -> Self {
        Self {
            hot,
            access,
            registry_map,
            registry_mode_map,
            upstream_map,
            cargo_index_map,
            repo_signer_map,
            vuln_db_map,
            config_path,
            config_change_repo,
            hot_reload_enabled,
            pending: Mutex::new(None),
            builder,
            banner,
        }
    }

    /// Discards the pending reload without applying. Returns `true` if one existed.
    pub fn discard_pending(&self) -> bool {
        self.pending
            .lock()
            .expect("pending reload lock poisoned")
            .take()
            .is_some()
    }

    /// Returns a non-sensitive snapshot of the pending reload for the GET endpoint.
    pub fn pending_snapshot(&self) -> Option<PendingReloadSnapshot> {
        self.pending
            .lock()
            .expect("pending reload lock poisoned")
            .as_ref()
            .map(|p| PendingReloadSnapshot {
                id: p.id,
                created_at: p.created_at,
                expires_at: p.expires_at,
                source: p.source.clone(),
                diff: p.diff.clone(),
            })
    }

    /// Drops the pending reload if it has passed its expiry time.
    pub fn expire_pending_if_stale(&self) {
        let mut guard = self.pending.lock().expect("pending reload lock poisoned");
        if let Some(ref p) = *guard {
            if Utc::now() > p.expires_at {
                *guard = None;
                tracing::info!("pending reload expired and was discarded");
            }
        }
    }

    /// Convenience: `load_pending` + `apply` atomically (for the immediate reload endpoint).
    pub async fn reload_immediate(&self, triggered_by: &str) -> Result<ReloadDiff, anyhow::Error> {
        let diff = self.load_pending(ReloadSource::AdminRequest).await?;
        self.apply(triggered_by).await?;
        Ok(diff)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub(super) mod tests;
