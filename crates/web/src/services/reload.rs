use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use batlehub_config::load as load_config;
use batlehub_core::{
    entities::{BannerLevel, GlobalBanner},
    services::{HotConfig, HotConfigLock},
};
use sqlx::PgPool;

use crate::{
    services::banner::BannerService, AccessConfig, AccessConfigLock, CargoIndexMap, RegistryMap,
    RegistryModeMap, UpstreamMap,
};

const PENDING_TTL_SECS: i64 = 600; // 10 minutes

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
    /// Pre-built, ready to swap in — no async work required during apply.
    pub new_hot: HotConfig,
    pub new_access: AccessConfig,
    pub new_registry_map: RegistryMap,
    pub new_registry_mode_map: RegistryModeMap,
    pub new_upstream_map: UpstreamMap,
    pub new_cargo_index_map: CargoIndexMap,
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
        )> + Send
        + Sync,
>;

/// Service responsible for hot-reloading configuration at runtime.
pub struct ConfigReloadService {
    hot: HotConfigLock,
    access: AccessConfigLock,
    registry_map: RegistryMap,
    registry_mode_map: RegistryModeMap,
    upstream_map: UpstreamMap,
    cargo_index_map: CargoIndexMap,
    config_path: String,
    pool: Option<PgPool>,
    pub hot_reload_enabled: bool,
    pending: Mutex<Option<PendingReload>>,
    builder: HotConfigBuilder,
    banner: Option<Arc<BannerService>>,
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
        config_path: String,
        pool: Option<PgPool>,
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
            config_path,
            pool,
            hot_reload_enabled,
            pending: Mutex::new(None),
            builder,
            banner,
        }
    }

    /// Re-reads the config file, validates, and builds a new HotConfig + AccessConfig.
    /// Stores the result as a pending reload (does NOT apply).
    /// Replaces any existing pending reload.
    pub async fn load_pending(&self, source: ReloadSource) -> Result<ReloadDiff, anyhow::Error> {
        if !self.hot_reload_enabled {
            anyhow::bail!("hot reload is disabled (BATLEHUB_DISABLE_HOT_RELOAD=1)");
        }
        let new_config = load_config(&self.config_path)?;
        let (
            new_hot,
            new_access,
            new_registry_map,
            new_registry_mode_map,
            new_upstream_map,
            new_cargo_index_map,
        ) = (self.builder)(&new_config)?;
        let diff = self.compute_diff(&new_hot, &new_access).await;
        let now = Utc::now();
        let pending = PendingReload {
            id: Uuid::new_v4(),
            created_at: now,
            expires_at: now + chrono::Duration::seconds(PENDING_TTL_SECS),
            source,
            diff: diff.clone(),
            new_hot,
            new_access,
            new_registry_map,
            new_registry_mode_map,
            new_upstream_map,
            new_cargo_index_map,
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

        // Clear the in-progress banner on success.
        if let Some(ref banner) = self.banner {
            let _ = banner.clear().await;
        }

        // Persist audit row (best-effort — do not fail the reload if DB is unavailable).
        self.persist_audit(&pending.diff, triggered_by, "applied", None)
            .await;

        Ok(pending.diff)
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

    // ── Internals ─────────────────────────────────────────────────────────────

    async fn compute_diff(&self, new_hot: &HotConfig, new_access: &AccessConfig) -> ReloadDiff {
        let old_hot = self.hot.read().await;
        let old_names: std::collections::HashSet<&str> =
            old_hot.registries.keys().map(String::as_str).collect();
        let new_names: std::collections::HashSet<&str> =
            new_hot.registries.keys().map(String::as_str).collect();

        let added: Vec<String> = new_names
            .difference(&old_names)
            .map(|s| s.to_string())
            .collect();
        let removed: Vec<String> = old_names
            .difference(&new_names)
            .map(|s| s.to_string())
            .collect();
        let limits_changed = old_hot.max_artifact_size_bytes != new_hot.max_artifact_size_bytes;

        // AccessConfig doesn't implement PartialEq (HashSet comparison is cheap but
        // we don't want to derive it on a potentially large struct). Mark as changed
        // conservatively whenever the registry set differs or limits change.
        let access_config_changed = !added.is_empty() || !removed.is_empty() || limits_changed || {
            let old_access = self.access.read().await;
            old_access.anonymous != new_access.anonymous
                || old_access.user != new_access.user
                || old_access.admin != new_access.admin
        };

        ReloadDiff {
            added_registries: added,
            removed_registries: removed,
            changed_registries: vec![],
            access_config_changed,
            limits_changed,
        }
    }

    async fn persist_audit(
        &self,
        diff: &ReloadDiff,
        triggered_by: &str,
        status: &str,
        error_msg: Option<&str>,
    ) {
        let pool = match self.pool.as_ref() {
            Some(p) => p,
            None => return,
        };
        let diff_json = match serde_json::to_value(diff) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize diff for audit");
                return;
            }
        };
        let added = diff.added_registries.len();
        let removed = diff.removed_registries.len();
        let summary = format!(
            "{} registr{} added, {} removed",
            added,
            if added == 1 { "y" } else { "ies" },
            removed
        );
        let result = sqlx::query(
            "INSERT INTO config_changes (id, triggered_by, triggered_at, status, diff, summary, error_msg)
             VALUES ($1, $2, NOW(), $3, $4, $5, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(triggered_by)
        .bind(status)
        .bind(diff_json)
        .bind(&summary)
        .bind(error_msg)
        .execute(pool)
        .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "failed to persist config change audit row");
        }
    }
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use batlehub_core::services::new_hot_lock;

    fn make_svc(enabled: bool) -> Arc<ConfigReloadService> {
        let hot = new_hot_lock(batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        });
        let access = crate::new_access_lock(crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        });
        let builder: HotConfigBuilder =
            Arc::new(|_| anyhow::bail!("builder not used in unit tests"));
        Arc::new(ConfigReloadService::new(
            hot,
            access,
            crate::RegistryMap::new(HashMap::new()),
            crate::RegistryModeMap::new(HashMap::new()),
            crate::UpstreamMap::new(HashMap::new()),
            crate::CargoIndexMap::new(HashMap::new()),
            "config.toml".to_owned(),
            None,
            enabled,
            builder,
            None,
        ))
    }

    #[tokio::test]
    async fn load_pending_returns_error_when_disabled() {
        let svc = make_svc(false);
        let err = svc
            .load_pending(ReloadSource::AdminRequest)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disabled"));
    }

    #[tokio::test]
    async fn apply_returns_error_when_disabled() {
        let svc = make_svc(false);
        let err = svc.apply("test").await.unwrap_err();
        assert!(err.to_string().contains("disabled"));
    }

    #[tokio::test]
    async fn apply_returns_error_when_no_pending() {
        let svc = make_svc(true);
        let err = svc.apply("test").await.unwrap_err();
        assert!(err.to_string().contains("no pending"));
    }

    #[test]
    fn discard_returns_false_when_nothing_pending() {
        let svc = make_svc(true);
        assert!(!svc.discard_pending());
    }

    #[test]
    fn pending_snapshot_is_none_initially() {
        let svc = make_svc(true);
        assert!(svc.pending_snapshot().is_none());
    }

    #[test]
    fn discard_returns_true_when_pending_exists() {
        let svc = make_svc(true);
        // Inject a pending reload manually via the mutex
        let hot = batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        };
        let access = crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        };
        let pending = PendingReload {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(600),
            source: ReloadSource::AdminRequest,
            diff: ReloadDiff::default(),
            new_hot: hot,
            new_access: access,
            new_registry_map: crate::RegistryMap::new(HashMap::new()),
            new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
            new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        };
        *svc.pending.lock().unwrap() = Some(pending);

        assert!(svc.discard_pending());
        assert!(svc.pending_snapshot().is_none());
        assert!(!svc.discard_pending()); // second call returns false
    }

    #[test]
    fn expire_stale_clears_expired_pending() {
        let svc = make_svc(true);
        let hot = batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        };
        let access = crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        };
        let expired = PendingReload {
            id: Uuid::new_v4(),
            created_at: Utc::now() - chrono::Duration::seconds(700),
            expires_at: Utc::now() - chrono::Duration::seconds(100), // already expired
            source: ReloadSource::FileWatcher,
            diff: ReloadDiff::default(),
            new_hot: hot,
            new_access: access,
            new_registry_map: crate::RegistryMap::new(HashMap::new()),
            new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
            new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        };
        *svc.pending.lock().unwrap() = Some(expired);

        svc.expire_pending_if_stale();
        assert!(svc.pending_snapshot().is_none());
    }

    #[tokio::test]
    async fn apply_success_swaps_hot_config() {
        let svc = make_svc(true);
        let new_hot = batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: Some(42),
        };
        let new_access = crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        };
        let pending = PendingReload {
            id: Uuid::new_v4(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::seconds(600),
            source: ReloadSource::AdminRequest,
            diff: ReloadDiff {
                added_registries: vec!["new-reg".to_string()],
                ..Default::default()
            },
            new_hot,
            new_access,
            new_registry_map: crate::RegistryMap::new(HashMap::new()),
            new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
            new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        };
        *svc.pending.lock().unwrap() = Some(pending);

        let diff = svc.apply("test-user").await.unwrap();

        assert_eq!(diff.added_registries, vec!["new-reg"]);
        assert!(svc.pending_snapshot().is_none());
        let hot = svc.hot.read().await;
        assert_eq!(hot.max_artifact_size_bytes, Some(42));
    }

    #[tokio::test]
    async fn reload_immediate_applies_config() {
        let tmp_path = format!("/tmp/batlehub_reload_test_{}.toml", Uuid::new_v4());
        std::fs::write(
            &tmp_path,
            "[server]\nhost = \"127.0.0.1\"\nport = 8080\n\n[database]\ntype = \"postgresql\"\nurl = \"postgresql://user:pass@localhost/db\"\n\n[storage]\ntype = \"filesystem\"\npath = \"./tmp\"\n",
        )
        .unwrap();

        let hot = batlehub_core::services::new_hot_lock(batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        });
        let access = crate::new_access_lock(crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        });
        let builder: HotConfigBuilder = Arc::new(|_| {
            Ok((
                batlehub_core::services::HotConfig {
                    registries: HashMap::new(),
                    policies: HashMap::new(),
                    versioning: HashMap::new(),
                    signing: HashMap::new(),
                    sbom: HashMap::new(),
                    beta_channel: HashMap::new(),
                    max_artifact_size_bytes: Some(999),
                },
                crate::AccessConfig {
                    anonymous: Default::default(),
                    user: Default::default(),
                    admin: Default::default(),
                    groups: Default::default(),
                    explore_anonymous: Default::default(),
                    explore_user: Default::default(),
                    explore_admin: Default::default(),
                },
                crate::RegistryMap::new(HashMap::new()),
                crate::RegistryModeMap::new(HashMap::new()),
                crate::UpstreamMap::new(HashMap::new()),
                crate::CargoIndexMap::new(HashMap::new()),
            ))
        });
        let svc = Arc::new(ConfigReloadService::new(
            hot,
            access,
            crate::RegistryMap::new(HashMap::new()),
            crate::RegistryModeMap::new(HashMap::new()),
            crate::UpstreamMap::new(HashMap::new()),
            crate::CargoIndexMap::new(HashMap::new()),
            tmp_path.clone(),
            None,
            true,
            builder,
            None,
        ));

        let diff = svc.reload_immediate("test").await.unwrap();
        assert!(diff.added_registries.is_empty());
        assert!(svc.pending_snapshot().is_none());
        let hot = svc.hot.read().await;
        assert_eq!(hot.max_artifact_size_bytes, Some(999));

        let _ = std::fs::remove_file(tmp_path);
    }

    #[tokio::test]
    async fn list_changes_returns_error_without_database() {
        let svc = make_svc(true);
        let err = svc.list_changes(0, 10).await.unwrap_err();
        assert!(err.to_string().contains("database not configured"));
    }

    #[tokio::test]
    async fn apply_expired_pending_returns_error() {
        let svc = make_svc(true);
        let hot = batlehub_core::services::HotConfig {
            registries: HashMap::new(),
            policies: HashMap::new(),
            versioning: HashMap::new(),
            signing: HashMap::new(),
            sbom: HashMap::new(),
            beta_channel: HashMap::new(),
            max_artifact_size_bytes: None,
        };
        let access = crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        };
        let expired = PendingReload {
            id: Uuid::new_v4(),
            created_at: Utc::now() - chrono::Duration::seconds(700),
            expires_at: Utc::now() - chrono::Duration::seconds(1),
            source: ReloadSource::AdminRequest,
            diff: ReloadDiff::default(),
            new_hot: hot,
            new_access: access,
            new_registry_map: crate::RegistryMap::new(HashMap::new()),
            new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
            new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        };
        *svc.pending.lock().unwrap() = Some(expired);

        let err = svc.apply("test").await.unwrap_err();
        assert!(err.to_string().contains("expired"), "got: {err}");
    }
}
