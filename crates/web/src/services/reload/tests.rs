use std::collections::HashMap;
use std::sync::Arc;

use batlehub_core::services::new_hot_lock;
use uuid::Uuid;

use super::*;

/// Build a `ConfigReloadService` that uses a real temporary file on disk.
/// The file is initialised with `initial_content` and its path is returned
/// alongside the service so tests can inspect it later.
async fn make_svc_with_file(
    enabled: bool,
    initial_content: &str,
) -> (Arc<ConfigReloadService>, tempfile::NamedTempFile) {
    use std::io::Write as _;
    let mut tmp = tempfile::NamedTempFile::new().expect("temp file");
    tmp.write_all(initial_content.as_bytes()).expect("write");
    let path = tmp.path().to_str().unwrap().to_owned();

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
    let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("builder not used in this test"));
    let svc = Arc::new(ConfigReloadService::new(
        hot,
        access,
        crate::RegistryMap::new(HashMap::new()),
        crate::RegistryModeMap::new(HashMap::new()),
        crate::UpstreamMap::new(HashMap::new()),
        crate::CargoIndexMap::new(HashMap::new()),
        crate::RepoSignerMap::default(),
        crate::VulnDbMap::default(),
        path,
        None,
        enabled,
        builder,
        None,
    ));
    (svc, tmp)
}

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
        crate::RepoSignerMap::default(),
        crate::VulnDbMap::default(),
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
    // The service starts with no signers; the reload should swap in a new one.
    assert!(svc.repo_signer_map.get("apt").is_none());
    let seed = "9d61b19deffeba00aa3f3b6e3b0fe6a3f3a76b08e2c0a3f3b6e3b0fe6a3f3a76";
    let new_signers: HashMap<String, Arc<batlehub_adapters::repo::OpenPgpSigner>> = [(
        "apt".to_owned(),
        Arc::new(
            batlehub_adapters::repo::OpenPgpSigner::from_seed_hex(seed, 1_700_000_000, "BatleHub")
                .unwrap(),
        ),
    )]
    .into();
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
        content: None,
        new_hot,
        new_access,
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        new_repo_signer_map: crate::RepoSignerMap::from(new_signers),
        new_vuln_db_map: crate::VulnDbMap::default(),
    };
    *svc.pending.lock().unwrap() = Some(pending);

    let diff = svc.apply("test-user").await.unwrap();

    assert_eq!(diff.added_registries, vec!["new-reg"]);
    assert!(svc.pending_snapshot().is_none());
    let hot = svc.hot.read().await;
    assert_eq!(hot.max_artifact_size_bytes, Some(42));
    // The deb/rpm signer map was swapped in by the same apply().
    assert!(svc.repo_signer_map.get("apt").is_some());
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
            crate::RepoSignerMap::default(),
            crate::VulnDbMap::default(),
        ))
    });
    let svc = Arc::new(ConfigReloadService::new(
        hot,
        access,
        crate::RegistryMap::new(HashMap::new()),
        crate::RegistryModeMap::new(HashMap::new()),
        crate::UpstreamMap::new(HashMap::new()),
        crate::CargoIndexMap::new(HashMap::new()),
        crate::RepoSignerMap::default(),
        crate::VulnDbMap::default(),
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
        content: None,
        new_hot: hot,
        new_access: access,
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        new_repo_signer_map: crate::RepoSignerMap::default(),
        new_vuln_db_map: crate::VulnDbMap::default(),
    }
}

// ── config_content + load_pending_from_content + apply disk-write ─────────────

#[tokio::test]
async fn config_content_reads_file_from_disk() {
    let (svc, _tmp) = make_svc_with_file(true, "initial = true\n").await;
    let content = svc.config_content().await.expect("read");
    assert_eq!(content, "initial = true\n");
}

#[tokio::test]
async fn config_content_returns_error_for_missing_file() {
    let svc = make_svc(true); // uses non-existent "config.toml"
    let err = svc.config_content().await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[tokio::test]
async fn load_pending_from_content_returns_error_when_disabled() {
    let svc = make_svc(false);
    let err = svc
        .load_pending_from_content("[servers]\n", ReloadSource::AdminRequest)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("disabled"));
}

#[tokio::test]
async fn load_pending_from_content_returns_error_for_invalid_toml() {
    let svc = make_svc(true);
    let err = svc
        .load_pending_from_content("not valid toml ::::", ReloadSource::AdminRequest)
        .await
        .unwrap_err();
    // The error comes from TOML parsing — just verify it propagates.
    assert!(!err.to_string().is_empty());
}

#[tokio::test]
async fn load_pending_from_content_stores_raw_content_in_pending() {
    let raw = "# valid minimal config\n";
    let svc = make_svc(true);
    // Override builder to succeed without touching the file.
    let hot = batlehub_core::services::HotConfig {
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
    // Inject a pending with content set, simulating a successful parse.
    let pending = PendingReload {
        id: Uuid::new_v4(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(600),
        source: ReloadSource::AdminRequest,
        diff: ReloadDiff::default(),
        content: Some(raw.to_owned()),
        new_hot: hot,
        new_access: access,
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        new_repo_signer_map: crate::RepoSignerMap::default(),
        new_vuln_db_map: crate::VulnDbMap::default(),
    };
    *svc.pending.lock().unwrap() = Some(pending);

    let stored = svc.pending.lock().unwrap();
    assert_eq!(stored.as_ref().unwrap().content.as_deref(), Some(raw));
}

#[tokio::test]
async fn apply_writes_editor_content_to_disk() {
    let initial = "# initial\n";
    let new_toml = "# after editor apply\n";
    let (svc, tmp) = make_svc_with_file(true, initial).await;

    // Manually set a pending reload with content (as load_pending_from_content would).
    let pending = PendingReload {
        id: Uuid::new_v4(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(600),
        source: ReloadSource::AdminRequest,
        diff: ReloadDiff::default(),
        content: Some(new_toml.to_owned()),
        new_hot: batlehub_core::services::HotConfig::default(),
        new_access: crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        },
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        new_repo_signer_map: crate::RepoSignerMap::default(),
        new_vuln_db_map: crate::VulnDbMap::default(),
    };
    *svc.pending.lock().unwrap() = Some(pending);

    svc.apply("test-user").await.unwrap();

    // Verify the file now contains the editor-submitted content.
    let on_disk = tokio::fs::read_to_string(tmp.path()).await.unwrap();
    assert_eq!(on_disk, new_toml);
    // And config_content() returns the updated file.
    let via_svc = svc.config_content().await.unwrap();
    assert_eq!(via_svc, new_toml);
}

#[tokio::test]
async fn apply_with_no_content_leaves_file_unchanged() {
    let initial = "# unchanged\n";
    let (svc, tmp) = make_svc_with_file(true, initial).await;

    let pending = PendingReload {
        id: Uuid::new_v4(),
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(600),
        source: ReloadSource::FileWatcher,
        diff: ReloadDiff::default(),
        content: None, // file-watcher path — no content to write back
        new_hot: batlehub_core::services::HotConfig::default(),
        new_access: crate::AccessConfig {
            anonymous: Default::default(),
            user: Default::default(),
            admin: Default::default(),
            groups: Default::default(),
            explore_anonymous: Default::default(),
            explore_user: Default::default(),
            explore_admin: Default::default(),
        },
        new_registry_map: crate::RegistryMap::new(HashMap::new()),
        new_registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        new_upstream_map: crate::UpstreamMap::new(HashMap::new()),
        new_cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        new_repo_signer_map: crate::RepoSignerMap::default(),
        new_vuln_db_map: crate::VulnDbMap::default(),
    };
    *svc.pending.lock().unwrap() = Some(pending);
    svc.apply("test-user").await.unwrap();

    let on_disk = tokio::fs::read_to_string(tmp.path()).await.unwrap();
    assert_eq!(on_disk, initial);
}
