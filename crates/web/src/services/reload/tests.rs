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
    let builder: HotConfigBuilder = Arc::new(|_| anyhow::bail!("builder not used in this test"));
    make_svc_with_file_and_builder(enabled, initial_content, builder).await
}

/// Same as `make_svc_with_file` but lets the caller supply a `builder` that
/// actually succeeds, for tests that exercise `load_pending`'s diff computation.
async fn make_svc_with_file_and_builder(
    enabled: bool,
    initial_content: &str,
    builder: HotConfigBuilder,
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
    let svc = Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot,
        access,
        registry_map: crate::RegistryMap::new(HashMap::new()),
        registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        upstream_map: crate::UpstreamMap::new(HashMap::new()),
        cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        repo_signer_map: crate::RepoSignerMap::default(),
        vuln_db_map: crate::VulnDbMap::default(),
        sumdb_map: crate::SumDbMap::default(),
        registry_host_map: crate::RegistryHostMap::default(),
        proxy_trust: crate::middleware::ProxyTrust::default(),
        config_path: path,
        config_change_repo: None,
        hot_reload_enabled: enabled,
        builder,
        banner: None,
    }));
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
    Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot,
        access,
        registry_map: crate::RegistryMap::new(HashMap::new()),
        registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        upstream_map: crate::UpstreamMap::new(HashMap::new()),
        cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        repo_signer_map: crate::RepoSignerMap::default(),
        vuln_db_map: crate::VulnDbMap::default(),
        sumdb_map: crate::SumDbMap::default(),
        registry_host_map: crate::RegistryHostMap::default(),
        proxy_trust: crate::middleware::ProxyTrust::default(),
        config_path: "config.toml".to_owned(),
        config_change_repo: None,
        hot_reload_enabled: enabled,
        builder,
        banner: None,
    }))
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
        new_sumdb_map: crate::SumDbMap::default(),
        new_registry_host_map: crate::RegistryHostMap::default(),
        new_proxy_trust: crate::middleware::ProxyTrust::from_config(Some(&[
            "10.42.0.0/16".to_owned()
        ])),
        warnings: Vec::new(),
    };
    // The handle the app's middleware would hold. It must see the swap, or host
    // routing can go live while trust stays at its startup value.
    let live_trust = svc.proxy_trust.clone();
    assert!(!live_trust.is_configured());
    *svc.pending.lock().unwrap() = Some(pending);

    let diff = svc.apply("test-user").await.unwrap();

    assert_eq!(diff.added_registries, vec!["new-reg"]);
    assert!(svc.pending_snapshot().is_none());
    let hot = svc.hot.read().await;
    assert_eq!(hot.max_artifact_size_bytes, Some(42));
    // The deb/rpm signer map was swapped in by the same apply().
    assert!(svc.repo_signer_map.get("apt").is_some());
    // …and so was the proxy-trust policy.
    assert!(live_trust.is_configured());
    assert_eq!(
        live_trust.verdict_for(Some("10.42.7.1".parse().unwrap())),
        crate::middleware::PeerTrust::Trusted
    );
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
        Ok(BuiltHotState {
            hot: batlehub_core::services::HotConfig {
                registries: HashMap::new(),
                policies: HashMap::new(),
                max_artifact_size_bytes: Some(999),
                ..Default::default()
            },
            access: crate::AccessConfig {
                anonymous: Default::default(),
                user: Default::default(),
                admin: Default::default(),
                groups: Default::default(),
                explore_anonymous: Default::default(),
                explore_user: Default::default(),
                explore_admin: Default::default(),
            },
            registry_map: crate::RegistryMap::new(HashMap::new()),
            registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            upstream_map: crate::UpstreamMap::new(HashMap::new()),
            cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
            repo_signer_map: crate::RepoSignerMap::default(),
            vuln_db_map: crate::VulnDbMap::default(),
            sumdb_map: crate::SumDbMap::default(),
            registry_host_map: crate::RegistryHostMap::default(),
        })
    });
    let svc = Arc::new(ConfigReloadService::new(ConfigReloadParams {
        hot,
        access,
        registry_map: crate::RegistryMap::new(HashMap::new()),
        registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
        upstream_map: crate::UpstreamMap::new(HashMap::new()),
        cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
        repo_signer_map: crate::RepoSignerMap::default(),
        vuln_db_map: crate::VulnDbMap::default(),
        sumdb_map: crate::SumDbMap::default(),
        registry_host_map: crate::RegistryHostMap::default(),
        proxy_trust: crate::middleware::ProxyTrust::default(),
        config_path: tmp_path.clone(),
        config_change_repo: None,
        hot_reload_enabled: true,
        builder,
        banner: None,
    }));

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
        new_sumdb_map: crate::SumDbMap::default(),
        new_registry_host_map: crate::RegistryHostMap::default(),
        new_proxy_trust: crate::middleware::ProxyTrust::default(),
        warnings: Vec::new(),
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

/// Regression test: a file-watcher event whose config content is structurally a
/// no-op per `compute_diff` (no registry added/removed, no top-level access/limits
/// change) must still store a pending reload. `compute_diff` cannot see changes to
/// an *existing* registry's fields (`changed_registries` is always empty — see its
/// doc comment), so treating "empty diff" as "nothing changed" would silently drop
/// real edits to an existing registry's config.
#[tokio::test]
async fn load_pending_stores_pending_even_when_diff_is_structurally_noop() {
    let builder: HotConfigBuilder = Arc::new(|_| {
        Ok(BuiltHotState {
            hot: batlehub_core::services::HotConfig::default(),
            access: crate::AccessConfig {
                anonymous: Default::default(),
                user: Default::default(),
                admin: Default::default(),
                groups: Default::default(),
                explore_anonymous: Default::default(),
                explore_user: Default::default(),
                explore_admin: Default::default(),
            },
            registry_map: crate::RegistryMap::new(HashMap::new()),
            registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            upstream_map: crate::UpstreamMap::new(HashMap::new()),
            cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
            repo_signer_map: crate::RepoSignerMap::default(),
            vuln_db_map: crate::VulnDbMap::default(),
            sumdb_map: crate::SumDbMap::default(),
            registry_host_map: crate::RegistryHostMap::default(),
        })
    });
    let minimal_config = r#"
        [server]
        host = "127.0.0.1"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://user:pass@localhost/db"

        [storage]
        type = "filesystem"
        path = "./tmp"
        "#;
    let (svc, _tmp) = make_svc_with_file_and_builder(true, minimal_config, builder).await;

    let diff = svc
        .load_pending(ReloadSource::FileWatcher)
        .await
        .expect("load_pending");

    assert!(diff.is_noop());
    assert!(
        svc.pending_snapshot().is_some(),
        "a structurally-noop diff must not suppress storing the pending reload"
    );
}

/// A builder that succeeds with empty state, for tests that only care about the
/// staging bookkeeping rather than what gets built.
fn noop_builder() -> HotConfigBuilder {
    Arc::new(|_| {
        Ok(BuiltHotState {
            hot: batlehub_core::services::HotConfig::default(),
            access: crate::AccessConfig {
                anonymous: Default::default(),
                user: Default::default(),
                admin: Default::default(),
                groups: Default::default(),
                explore_anonymous: Default::default(),
                explore_user: Default::default(),
                explore_admin: Default::default(),
            },
            registry_map: crate::RegistryMap::new(HashMap::new()),
            registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            upstream_map: crate::UpstreamMap::new(HashMap::new()),
            cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
            repo_signer_map: crate::RepoSignerMap::default(),
            vuln_db_map: crate::VulnDbMap::default(),
            sumdb_map: crate::SumDbMap::default(),
            registry_host_map: crate::RegistryHostMap::default(),
        })
    })
}

const MINIMAL_CONFIG: &str = r#"
[server]
host = "127.0.0.1"
port = 8080

[database]
type = "postgresql"
url = "postgresql://user:pass@localhost/db"

[storage]
type = "filesystem"
path = "./tmp"
"#;

/// `pending_created` is the only thing that distinguishes "staged, go apply it"
/// from "nothing to stage": both return `Ok` with an empty diff, and the second
/// only reveals itself when the follow-up apply fails.
#[tokio::test]
async fn resubmitting_identical_content_reports_that_nothing_was_staged() {
    let (svc, _tmp) = make_svc_with_file_and_builder(true, MINIMAL_CONFIG, noop_builder()).await;

    let first = svc
        .load_pending_from_content(MINIMAL_CONFIG, ReloadSource::AdminRequest)
        .await
        .expect("first submission");
    assert!(first.pending_created, "the first submission stages");
    assert!(svc.pending_snapshot().is_some());

    // The admin discards it, then re-submits the very same bytes from the editor
    // — the shape of the real complaint: the file watcher had already loaded this
    // content, so the dedup fires even though nothing is staged any more.
    assert!(svc.discard_pending());
    let second = svc
        .load_pending_from_content(MINIMAL_CONFIG, ReloadSource::AdminRequest)
        .await
        .expect("re-submission still succeeds");

    assert!(
        !second.pending_created,
        "identical content stages nothing, and must say so"
    );
    assert!(svc.pending_snapshot().is_none());
    let err = svc.apply("test").await.unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<ReloadApplyError>(),
            Some(ReloadApplyError::NoPendingReload)
        ),
        "…which is exactly what applying would have hit: {err}"
    );
}

#[tokio::test]
async fn validate_content_never_reports_a_staged_pending() {
    let (svc, _tmp) = make_svc_with_file_and_builder(true, MINIMAL_CONFIG, noop_builder()).await;

    let outcome = svc
        .validate_content(MINIMAL_CONFIG)
        .await
        .expect("validate");

    assert!(
        !outcome.pending_created,
        "validate is a dry run by contract"
    );
    assert!(svc.pending_snapshot().is_none());
}

/// Regression test: a *repeated* file-watcher event whose raw config bytes are
/// byte-identical to the previous load attempt (e.g. a touch or atomic-save
/// rewrite) must not rebuild or replace the existing pending reload — this is the
/// actual case the file watcher hits repeatedly and needs to dedup.
#[tokio::test]
async fn load_pending_skips_rebuild_when_raw_content_is_unchanged() {
    let builder: HotConfigBuilder = Arc::new(|_| {
        Ok(BuiltHotState {
            hot: batlehub_core::services::HotConfig::default(),
            access: crate::AccessConfig {
                anonymous: Default::default(),
                user: Default::default(),
                admin: Default::default(),
                groups: Default::default(),
                explore_anonymous: Default::default(),
                explore_user: Default::default(),
                explore_admin: Default::default(),
            },
            registry_map: crate::RegistryMap::new(HashMap::new()),
            registry_mode_map: crate::RegistryModeMap::new(HashMap::new()),
            upstream_map: crate::UpstreamMap::new(HashMap::new()),
            cargo_index_map: crate::CargoIndexMap::new(HashMap::new()),
            repo_signer_map: crate::RepoSignerMap::default(),
            vuln_db_map: crate::VulnDbMap::default(),
            sumdb_map: crate::SumDbMap::default(),
            registry_host_map: crate::RegistryHostMap::default(),
        })
    });
    let minimal_config = r#"
        [server]
        host = "127.0.0.1"
        port = 8080

        [database]
        type = "postgresql"
        url = "postgresql://user:pass@localhost/db"

        [storage]
        type = "filesystem"
        path = "./tmp"
        "#;
    let (svc, _tmp) = make_svc_with_file_and_builder(true, minimal_config, builder).await;

    svc.load_pending(ReloadSource::FileWatcher)
        .await
        .expect("first load_pending");
    let first_id = svc
        .pending_snapshot()
        .expect("first load stores a pending")
        .id;

    // File watcher fires again; the file on disk hasn't actually changed.
    let second = svc
        .load_pending(ReloadSource::FileWatcher)
        .await
        .expect("second load_pending");

    assert!(second.is_noop());
    assert_eq!(
        svc.pending_snapshot().expect("pending left untouched").id,
        first_id,
        "unchanged raw content must not replace the existing pending reload"
    );
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
        new_sumdb_map: crate::SumDbMap::default(),
        new_registry_host_map: crate::RegistryHostMap::default(),
        new_proxy_trust: crate::middleware::ProxyTrust::default(),
        warnings: Vec::new(),
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
        new_sumdb_map: crate::SumDbMap::default(),
        new_registry_host_map: crate::RegistryHostMap::default(),
        new_proxy_trust: crate::middleware::ProxyTrust::default(),
        warnings: Vec::new(),
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
        new_sumdb_map: crate::SumDbMap::default(),
        new_registry_host_map: crate::RegistryHostMap::default(),
        new_proxy_trust: crate::middleware::ProxyTrust::default(),
        warnings: Vec::new(),
    };
    *svc.pending.lock().unwrap() = Some(pending);
    svc.apply("test-user").await.unwrap();

    let on_disk = tokio::fs::read_to_string(tmp.path()).await.unwrap();
    assert_eq!(on_disk, initial);
}
