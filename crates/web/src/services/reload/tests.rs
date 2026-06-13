use std::collections::HashMap;
use std::sync::Arc;

use batlehub_core::services::new_hot_lock;
use uuid::Uuid;

use super::*;

// ── Shared helper ─────────────────────────────────────────────────────────────

pub(super) fn make_svc(enabled: bool) -> Arc<ConfigReloadService> {
    let hot = new_hot_lock(batlehub_core::services::HotConfig {
        registries: HashMap::new(),
        policies: HashMap::new(),
        ..Default::default()
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
    let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("builder not used in unit tests"));
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

// ── Basic guard tests ─────────────────────────────────────────────────────────

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
    let pending = make_pending(600, false);
    *svc.pending.lock().unwrap() = Some(pending);

    assert!(svc.discard_pending());
    assert!(svc.pending_snapshot().is_none());
    assert!(!svc.discard_pending());
}

#[test]
fn expire_stale_clears_expired_pending() {
    let svc = make_svc(true);
    let expired = make_pending(-100, true);
    *svc.pending.lock().unwrap() = Some(expired);

    svc.expire_pending_if_stale();
    assert!(svc.pending_snapshot().is_none());
}

// ── Apply / reload tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn apply_success_swaps_hot_config() {
    let svc = make_svc(true);
    let new_hot = batlehub_core::services::HotConfig {
        registries: HashMap::new(),
        policies: HashMap::new(),
        max_artifact_size_bytes: Some(42),
        ..Default::default()
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
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(600),
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

    let hot = new_hot_lock(batlehub_core::services::HotConfig {
        registries: HashMap::new(),
        policies: HashMap::new(),
        ..Default::default()
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
                max_artifact_size_bytes: Some(999),
                ..Default::default()
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
    let expired = make_pending(-1, true);
    *svc.pending.lock().unwrap() = Some(expired);

    let err = svc.apply("test").await.unwrap_err();
    assert!(err.to_string().contains("expired"), "got: {err}");
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn make_pending(expires_offset_secs: i64, already_expired: bool) -> PendingReload {
    let hot = batlehub_core::services::HotConfig {
        registries: HashMap::new(),
        policies: HashMap::new(),
        ..Default::default()
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
    let created_at = if already_expired {
        chrono::Utc::now() - chrono::Duration::seconds(700)
    } else {
        chrono::Utc::now()
    };
    PendingReload {
        id: Uuid::new_v4(),
        created_at,
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_offset_secs),
        source: if already_expired {
            ReloadSource::FileWatcher
        } else {
            ReloadSource::AdminRequest
        },
        diff: ReloadDiff::default(),
        new_hot: hot,
        new_access: access,
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
    }
}
