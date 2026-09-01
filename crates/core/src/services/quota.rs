use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    entities::Identity,
    error::CoreError,
    ports::{QuotaOutcome, QuotaRepository, QuotaUsage},
};

/// Quota limits for a single registry, sourced from config.
#[derive(Debug, Clone)]
pub struct RegistryQuotaConfig {
    pub max_storage_bytes_per_user: Option<u64>,
    pub max_packages_per_user: Option<u32>,
    /// Warn when usage exceeds this fraction of the limit (0.0–1.0).
    pub warn_threshold: f64,
    pub enforcement: QuotaEnforcement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaEnforcement {
    Block,
    Warn,
}

/// Result of a quota check. Returned from
/// `QuotaService::check_and_record_publish_at_tier`.
#[derive(Debug, Clone, Default)]
pub struct QuotaCheck {
    pub bytes_used: u64,
    pub bytes_limit: Option<u64>,
    pub packages_used: u32,
    pub packages_limit: Option<u32>,
    /// True when usage is approaching or has exceeded the warning threshold.
    pub warning: bool,
}

impl QuotaCheck {
    /// Build `X-Quota-*` response headers. Returns an empty vec when no quota
    /// is configured for the registry (i.e. both limits are `None`).
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        if self.bytes_limit.is_none() && self.packages_limit.is_none() {
            return Vec::new();
        }
        let mut h = Vec::new();
        h.push(("X-Quota-Storage-Used", self.bytes_used.to_string()));
        if let Some(limit) = self.bytes_limit {
            h.push(("X-Quota-Storage-Limit", limit.to_string()));
        }
        h.push(("X-Quota-Packages-Used", self.packages_used.to_string()));
        if let Some(limit) = self.packages_limit {
            h.push(("X-Quota-Packages-Limit", limit.to_string()));
        }
        if self.warning {
            h.push(("X-Quota-Warning", "approaching-limit".to_owned()));
        }
        h
    }
}

pub struct QuotaService {
    repo: Arc<dyn QuotaRepository>,
    /// Registry name → quota configuration. Only registries with quota configured.
    configs: HashMap<String, RegistryQuotaConfig>,
}

impl QuotaService {
    pub fn new(
        repo: Arc<dyn QuotaRepository>,
        configs: HashMap<String, RegistryQuotaConfig>,
    ) -> Self {
        Self { repo, configs }
    }

    /// Check whether a publish is allowed under the current quota, and if so,
    /// atomically record it. Returns `CoreError::QuotaExceeded` when
    /// `enforcement = Block` and a limit is exceeded.
    ///
    /// `tier_quota` is the quota a deeper tier declared.
    ///
    /// RFC 0015 §4.5 attaches `quota` to the resource hierarchy: a namespace or
    /// package may declare its own, and composition is **wholesale** — the
    /// deeper block replaces the registry's rather than merging with it, because
    /// the motivating case is a *narrower* limit deeper down and a field merge
    /// could only ever raise one.
    ///
    /// `None` means no tier below the registry declared a quota, which is every
    /// publish before phase 4 and most of them after it. The accounting is
    /// unchanged either way: §4.5 notes that per-subject limits resolved per tier
    /// *"need no new accounting — the same counter this function
    /// already maintains, with the limit looked up at the deepest tier that
    /// declares one"*. This is that lookup, and nothing else moves.
    pub async fn check_and_record_publish_at_tier(
        &self,
        identity: &Identity,
        registry: &str,
        bytes: u64,
        tier_quota: Option<&RegistryQuotaConfig>,
    ) -> Result<QuotaCheck, CoreError> {
        let config = match tier_quota.or_else(|| self.configs.get(registry)) {
            Some(c) => c,
            None => {
                // No quota configured for this registry — pass through.
                return Ok(QuotaCheck::default());
            }
        };

        let user_id = match &identity.user_id {
            Some(id) => id.clone(),
            // Anonymous users: enforce if limits exist, otherwise pass.
            None if config.max_storage_bytes_per_user.is_some()
                || config.max_packages_per_user.is_some() =>
            {
                return Err(CoreError::QuotaExceeded(
                    "anonymous users cannot publish to quota-gated registries".into(),
                ))
            }
            None => return Ok(QuotaCheck::default()),
        };

        // In `Block` mode, pass the real limits so the repository atomically
        // rejects the publish (rather than recording it) when it would exceed
        // either one — this closes the check-then-record race that existed
        // when the check and the write were two separate calls. In `Warn`
        // mode (or when no limit is configured), pass `None` so the publish
        // always records; the warning is computed from the returned totals.
        let (enforce_bytes, enforce_packages) = match config.enforcement {
            QuotaEnforcement::Block => (
                config.max_storage_bytes_per_user,
                config.max_packages_per_user,
            ),
            QuotaEnforcement::Warn => (None, None),
        };

        let outcome = self
            .repo
            .try_record_publish(&user_id, registry, bytes, enforce_bytes, enforce_packages)
            .await?;

        let (new_bytes, new_count) = match outcome {
            QuotaOutcome::Recorded {
                bytes_used,
                packages_used,
            } => (bytes_used, packages_used),
            QuotaOutcome::Exceeded {
                bytes_used,
                packages_used,
            } => {
                return Err(CoreError::QuotaExceeded(exceeded_message(
                    config,
                    registry,
                    bytes_used,
                    packages_used,
                )))
            }
        };

        if config.enforcement == QuotaEnforcement::Warn {
            warn_if_over_limit(config, registry, new_bytes, new_count);
        }

        // Build QuotaCheck with updated counts
        let warning = is_warning(
            new_bytes,
            config.max_storage_bytes_per_user,
            config.warn_threshold,
        ) || is_warning(
            new_count as u64,
            config.max_packages_per_user.map(|x| x as u64),
            config.warn_threshold,
        );

        Ok(QuotaCheck {
            bytes_used: new_bytes,
            bytes_limit: config.max_storage_bytes_per_user,
            packages_used: new_count,
            packages_limit: config.max_packages_per_user,
            warning,
        })
    }

    /// Undo a recorded publish (e.g. on storage failure after quota was recorded).
    pub async fn revoke_publish(
        &self,
        identity: &Identity,
        registry: &str,
        bytes: u64,
    ) -> Result<(), CoreError> {
        let Some(user_id) = &identity.user_id else {
            return Ok(());
        };
        if self.configs.contains_key(registry) {
            self.repo.revoke_publish(user_id, registry, bytes).await?;
        }
        Ok(())
    }

    pub async fn get_usage(&self, user_id: &str, registry: &str) -> Result<QuotaUsage, CoreError> {
        self.repo.get_usage(user_id, registry).await
    }

    pub async fn list_usage(&self, registry: Option<&str>) -> Result<Vec<QuotaUsage>, CoreError> {
        self.repo.list_usage(registry).await
    }

    pub async fn reset(&self, user_id: &str, registry: &str) -> Result<(), CoreError> {
        self.repo.reset_usage(user_id, registry).await
    }

    /// The configured storage limit (bytes per user) for a registry, or
    /// `None` if the registry has no quota configured, or no storage limit
    /// within its quota config.
    pub fn max_storage_bytes(&self, registry: &str) -> Option<u64> {
        self.configs.get(registry)?.max_storage_bytes_per_user
    }

    /// One user's usage against their limits, for every quota-gated registry in
    /// `registries`.
    ///
    /// Backs `GET /api/v1/me/quota` (RFC 0004 §4.2). Three things are decided
    /// here rather than by the caller:
    ///
    /// - **Registries with no quota are omitted.** A meter with no limit has
    ///   nothing to measure, and the widget does not render one.
    /// - **`registries` is the caller's accessible set**, intersected with the
    ///   quota-gated ones. A user should not learn the names of registries they
    ///   cannot reach from a quota listing.
    /// - **The threshold verdict is the server's.** `warn_threshold` lives in
    ///   config next to the limits, so recomputing it client-side is a second
    ///   copy of a rule that can drift from the one enforcement uses.
    pub async fn status_for_user(
        &self,
        user_id: &str,
        registries: &[String],
    ) -> Result<Vec<RegistryQuotaStatus>, CoreError> {
        let mut out = Vec::new();
        for registry in registries {
            let Some(config) = self.configs.get(registry) else {
                continue; // no quota here — nothing to measure
            };
            if config.max_storage_bytes_per_user.is_none() && config.max_packages_per_user.is_none()
            {
                continue; // a quota block with neither limit set is not a quota
            }

            let usage = self.repo.get_usage(user_id, registry).await?;
            let bytes_used = usage.bytes_published;
            let packages_used = usage.packages_count;

            // Per dimension, so a reader can see *which* threshold was crossed
            // (RFC 0004 §4.2). A registry whose versions are at 82% while its
            // storage sits at 68% is not "at 82%" — colouring both meters the
            // same would say it was.
            let bytes_state = dimension_state(
                bytes_used,
                config.max_storage_bytes_per_user,
                config.warn_threshold,
            );
            let packages_state = dimension_state(
                u64::from(packages_used),
                config.max_packages_per_user.map(u64::from),
                config.warn_threshold,
            );

            out.push(RegistryQuotaStatus {
                registry: registry.clone(),
                bytes_used,
                bytes_limit: config.max_storage_bytes_per_user,
                bytes_state,
                packages_used,
                packages_limit: config.max_packages_per_user,
                packages_state,
                warn_threshold_pct: (config.warn_threshold * 100.0).round().clamp(0.0, 100.0) as u8,
                // The row's own verdict is the worse of its dimensions: one
                // limit reached is enough to refuse the next publish.
                state: [bytes_state, packages_state]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or(QuotaState::Ok),
            });
        }
        out.sort_by(|a, b| a.registry.cmp(&b.registry));
        Ok(out)
    }
}

/// How close one user is to one registry's quota.
///
/// `Ord` runs from least to most urgent, so the worse of two dimensions is
/// `max()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuotaState {
    /// Below the warning threshold.
    Ok,
    /// At or past `warn_threshold_pct` of a limit, but publishes still succeed.
    Warning,
    /// A limit is reached. Under `enforcement = "block"` the next publish is
    /// refused; under `"warn"` it succeeds and is logged.
    AtLimit,
}

/// One registry's quota, as it applies to one user.
#[derive(Debug, Clone)]
pub struct RegistryQuotaStatus {
    pub registry: String,
    pub bytes_used: u64,
    pub bytes_limit: Option<u64>,
    /// This dimension's own verdict, or `None` when it has no limit.
    pub bytes_state: Option<QuotaState>,
    pub packages_used: u32,
    pub packages_limit: Option<u32>,
    /// This dimension's own verdict, or `None` when it has no limit.
    pub packages_state: Option<QuotaState>,
    /// The percentage of a limit at which [`QuotaState::Warning`] begins.
    pub warn_threshold_pct: u8,
    /// The worse of the two dimensions — what the registry as a whole is at.
    pub state: QuotaState,
}

/// One dimension's verdict. `None` in, `None` out: a dimension with no limit
/// has nothing to be near.
fn dimension_state(used: u64, limit: Option<u64>, threshold: f64) -> Option<QuotaState> {
    let max = limit?;
    Some(if used >= max {
        QuotaState::AtLimit
    } else if is_warning(used, Some(max), threshold) {
        QuotaState::Warning
    } else {
        QuotaState::Ok
    })
}

fn is_warning(used: u64, limit: Option<u64>, threshold: f64) -> bool {
    match limit {
        Some(max) if max > 0 => used as f64 / max as f64 >= threshold,
        _ => false,
    }
}

/// Which of the two limits the repository refused the publish over.
///
/// The repository reports "exceeded" without saying which dimension, so the
/// bytes limit is tested first and the packages limit is the remaining case —
/// the same order the enforcement arguments are passed in.
fn exceeded_message(
    config: &RegistryQuotaConfig,
    registry: &str,
    bytes_used: u64,
    packages_used: u32,
) -> String {
    if config
        .max_storage_bytes_per_user
        .is_some_and(|max| bytes_used > max)
    {
        format!(
            "storage quota exceeded for registry '{registry}': \
             {bytes_used} bytes used, limit is {}",
            config.max_storage_bytes_per_user.unwrap_or(0)
        )
    } else {
        format!(
            "package quota exceeded for registry '{registry}': \
             {packages_used} packages, limit is {}",
            config.max_packages_per_user.unwrap_or(0)
        )
    }
}

/// In `Warn` mode the publish always records even past the limit; log it
/// server-side the same way the old check-then-write path did, since nothing
/// rejected the request to make the operator aware otherwise.
fn warn_if_over_limit(
    config: &RegistryQuotaConfig,
    registry: &str,
    new_bytes: u64,
    new_count: u32,
) {
    if config
        .max_storage_bytes_per_user
        .is_some_and(|max| new_bytes > max)
    {
        let max = config.max_storage_bytes_per_user.unwrap_or(0);
        tracing::warn!(
            "storage quota exceeded for registry '{registry}': \
             {new_bytes} bytes used, limit is {max}"
        );
    }
    if config
        .max_packages_per_user
        .is_some_and(|max| new_count > max)
    {
        let max = config.max_packages_per_user.unwrap_or(0);
        tracing::warn!(
            "package quota exceeded for registry '{registry}': \
             {new_count} packages, limit is {max}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use async_trait::async_trait;

    use crate::{
        entities::{Identity, Role},
        ports::QuotaUsage,
    };

    struct MockQuotaRepo {
        usage: Mutex<(u64, u32)>,
    }

    impl MockQuotaRepo {
        fn new(bytes: u64, packages: u32) -> Arc<Self> {
            Arc::new(Self {
                usage: Mutex::new((bytes, packages)),
            })
        }
    }

    #[async_trait]
    impl QuotaRepository for MockQuotaRepo {
        async fn get_usage(&self, user_id: &str, registry: &str) -> Result<QuotaUsage, CoreError> {
            let (bytes, packages) = *self.usage.lock().unwrap();
            Ok(QuotaUsage {
                user_id: user_id.to_owned(),
                registry: registry.to_owned(),
                bytes_published: bytes,
                packages_count: packages,
            })
        }

        async fn record_publish(&self, _: &str, _: &str, bytes: u64) -> Result<(), CoreError> {
            let mut g = self.usage.lock().unwrap();
            g.0 += bytes;
            g.1 += 1;
            Ok(())
        }

        async fn try_record_publish(
            &self,
            _: &str,
            _: &str,
            bytes: u64,
            max_bytes: Option<u64>,
            max_packages: Option<u32>,
        ) -> Result<QuotaOutcome, CoreError> {
            let mut g = self.usage.lock().unwrap();
            let new_bytes = g.0 + bytes;
            let new_packages = g.1 + 1;
            let exceeded = max_bytes.is_some_and(|max| new_bytes > max)
                || max_packages.is_some_and(|max| new_packages > max);
            if exceeded {
                return Ok(QuotaOutcome::Exceeded {
                    bytes_used: new_bytes,
                    packages_used: new_packages,
                });
            }
            g.0 = new_bytes;
            g.1 = new_packages;
            Ok(QuotaOutcome::Recorded {
                bytes_used: new_bytes,
                packages_used: new_packages,
            })
        }

        async fn revoke_publish(&self, _: &str, _: &str, bytes: u64) -> Result<(), CoreError> {
            let mut g = self.usage.lock().unwrap();
            g.0 = g.0.saturating_sub(bytes);
            g.1 = g.1.saturating_sub(1);
            Ok(())
        }

        async fn reset_usage(&self, _: &str, _: &str) -> Result<(), CoreError> {
            *self.usage.lock().unwrap() = (0, 0);
            Ok(())
        }

        async fn list_usage(&self, _: Option<&str>) -> Result<Vec<QuotaUsage>, CoreError> {
            Ok(vec![])
        }
    }

    fn user(id: &str) -> Identity {
        Identity {
            user_id: Some(id.to_owned()),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        }
    }

    fn block_config(max_bytes: u64, max_pkgs: u32) -> RegistryQuotaConfig {
        RegistryQuotaConfig {
            max_storage_bytes_per_user: Some(max_bytes),
            max_packages_per_user: Some(max_pkgs),
            warn_threshold: 0.8,
            enforcement: QuotaEnforcement::Block,
        }
    }

    fn svc_with(config: RegistryQuotaConfig, bytes: u64, pkgs: u32) -> QuotaService {
        let mut configs = HashMap::new();
        configs.insert("cargo".into(), config);
        QuotaService::new(MockQuotaRepo::new(bytes, pkgs), configs)
    }

    #[test]
    fn is_warning_at_threshold() {
        assert!(is_warning(80, Some(100), 0.8));
        assert!(!is_warning(79, Some(100), 0.8));
        assert!(!is_warning(10, None, 0.8));
        assert!(!is_warning(10, Some(0), 0.8));
    }

    #[test]
    fn quota_check_headers_empty_without_limits() {
        let check = QuotaCheck {
            bytes_used: 100,
            bytes_limit: None,
            packages_used: 5,
            packages_limit: None,
            warning: false,
        };
        assert!(check.headers().is_empty());
    }

    #[test]
    fn quota_check_headers_includes_all_fields() {
        let check = QuotaCheck {
            bytes_used: 900,
            bytes_limit: Some(1000),
            packages_used: 9,
            packages_limit: Some(10),
            warning: true,
        };
        let headers = check.headers();
        let names: Vec<_> = headers.iter().map(|(k, _)| *k).collect();
        assert!(names.contains(&"X-Quota-Storage-Used"));
        assert!(names.contains(&"X-Quota-Storage-Limit"));
        assert!(names.contains(&"X-Quota-Packages-Used"));
        assert!(names.contains(&"X-Quota-Packages-Limit"));
        assert!(names.contains(&"X-Quota-Warning"));
    }

    #[tokio::test]
    async fn anonymous_rejected_when_limits_exist() {
        let svc = svc_with(block_config(1_000_000, 10), 0, 0);
        let result = svc
            .check_and_record_publish_at_tier(&Identity::anonymous(), "cargo", 100, None)
            .await;
        assert!(matches!(result, Err(CoreError::QuotaExceeded(_))));
    }

    #[tokio::test]
    async fn byte_limit_blocks_when_enforcement_is_block() {
        let svc = svc_with(block_config(1000, 100), 900, 1);
        let result = svc
            .check_and_record_publish_at_tier(&user("alice"), "cargo", 200, None)
            .await;
        assert!(matches!(result, Err(CoreError::QuotaExceeded(_))));
    }

    #[tokio::test]
    async fn no_quota_config_passes_through() {
        let svc = QuotaService::new(MockQuotaRepo::new(0, 0), HashMap::new());
        let check = svc
            .check_and_record_publish_at_tier(&user("alice"), "cargo", 100, None)
            .await
            .unwrap();
        assert!(check.headers().is_empty());
    }

    #[tokio::test]
    async fn warn_mode_allows_over_limit() {
        let svc = svc_with(
            RegistryQuotaConfig {
                max_storage_bytes_per_user: Some(1000),
                max_packages_per_user: None,
                warn_threshold: 0.8,
                enforcement: QuotaEnforcement::Warn,
            },
            900,
            1,
        );
        let check = svc
            .check_and_record_publish_at_tier(&user("alice"), "cargo", 200, None)
            .await
            .unwrap();
        assert!(check.warning);
    }

    #[tokio::test]
    async fn revoke_publish_decrements_usage_for_configured_registry() {
        let svc = svc_with(block_config(1_000_000, 100), 500, 2);
        svc.revoke_publish(&user("alice"), "cargo", 200)
            .await
            .unwrap();
        let usage = svc.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 300);
        assert_eq!(usage.packages_count, 1);
    }

    #[tokio::test]
    async fn revoke_publish_noop_for_unconfigured_registry() {
        let svc = QuotaService::new(
            MockQuotaRepo::new(500, 2) as Arc<dyn QuotaRepository>,
            HashMap::new(),
        );
        svc.revoke_publish(&user("alice"), "cargo", 200)
            .await
            .unwrap();
        let usage = svc.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 500);
    }

    #[tokio::test]
    async fn revoke_publish_noop_for_anonymous() {
        let svc = svc_with(block_config(1_000_000, 100), 500, 2);
        svc.revoke_publish(&Identity::anonymous(), "cargo", 200)
            .await
            .unwrap();
        let usage = svc.get_usage("any", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 500);
    }

    #[tokio::test]
    async fn get_usage_reflects_repo_state() {
        let svc = svc_with(block_config(1_000_000, 100), 1_024, 3);
        let usage = svc.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 1_024);
        assert_eq!(usage.packages_count, 3);
        assert_eq!(usage.user_id, "alice");
        assert_eq!(usage.registry, "cargo");
    }

    #[tokio::test]
    async fn list_usage_passes_through_to_repo() {
        let svc = svc_with(block_config(1_000_000, 100), 0, 0);
        let list = svc.list_usage(None).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn reset_zeroes_usage_for_user() {
        let svc = svc_with(block_config(1_000_000, 100), 5_000, 10);
        svc.reset("alice", "cargo").await.unwrap();
        let usage = svc.get_usage("alice", "cargo").await.unwrap();
        assert_eq!(usage.bytes_published, 0);
        assert_eq!(usage.packages_count, 0);
    }

    #[test]
    fn max_storage_bytes_returns_configured_limit() {
        let svc = svc_with(block_config(1_000_000, 100), 0, 0);
        assert_eq!(svc.max_storage_bytes("cargo"), Some(1_000_000));
    }

    #[test]
    fn max_storage_bytes_none_for_unconfigured_registry() {
        let svc = svc_with(block_config(1_000_000, 100), 0, 0);
        assert_eq!(svc.max_storage_bytes("npm"), None);
    }

    #[test]
    fn max_storage_bytes_none_when_only_package_limit_configured() {
        let mut configs = HashMap::new();
        configs.insert(
            "cargo".to_owned(),
            RegistryQuotaConfig {
                max_storage_bytes_per_user: None,
                max_packages_per_user: Some(100),
                warn_threshold: 0.8,
                enforcement: QuotaEnforcement::Block,
            },
        );
        let svc = QuotaService::new(MockQuotaRepo::new(0, 0), configs);
        assert_eq!(svc.max_storage_bytes("cargo"), None);
    }

    // ── status_for_user (RFC 0004 §4.2) ──────────────────────────────────────

    #[tokio::test]
    async fn status_for_user_omits_registries_without_a_quota() {
        let svc = svc_with(block_config(1_000, 10), 100, 1);
        let rows = svc
            .status_for_user("alice", &["cargo".to_owned(), "npm".to_owned()])
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "npm has no quota configured");
        assert_eq!(rows[0].registry, "cargo");
    }

    #[tokio::test]
    async fn status_for_user_omits_a_quota_block_with_no_limits() {
        let mut configs = HashMap::new();
        configs.insert(
            "cargo".to_owned(),
            RegistryQuotaConfig {
                max_storage_bytes_per_user: None,
                max_packages_per_user: None,
                warn_threshold: 0.8,
                enforcement: QuotaEnforcement::Warn,
            },
        );
        let svc = QuotaService::new(MockQuotaRepo::new(500, 5), configs);
        let rows = svc
            .status_for_user("alice", &["cargo".to_owned()])
            .await
            .unwrap();
        assert!(rows.is_empty(), "neither limit set is not a quota");
    }

    #[tokio::test]
    async fn status_for_user_never_reports_a_registry_the_caller_cannot_reach() {
        let svc = svc_with(block_config(1_000, 10), 100, 1);
        let rows = svc.status_for_user("alice", &[]).await.unwrap();
        assert!(
            rows.is_empty(),
            "the accessible set is the whole input; an empty one yields nothing"
        );
    }

    #[tokio::test]
    async fn status_for_user_reports_each_threshold_state() {
        for (bytes, expected) in [
            (0u64, QuotaState::Ok),
            (799, QuotaState::Ok),
            (800, QuotaState::Warning),
            (999, QuotaState::Warning),
            (1_000, QuotaState::AtLimit),
            (1_500, QuotaState::AtLimit),
        ] {
            let svc = svc_with(block_config(1_000, 10), bytes, 1);
            let rows = svc
                .status_for_user("alice", &["cargo".to_owned()])
                .await
                .unwrap();
            assert_eq!(
                rows[0].state, expected,
                "{bytes} bytes against a 1000 limit"
            );
            assert_eq!(rows[0].warn_threshold_pct, 80);
        }
    }

    #[tokio::test]
    async fn status_for_user_crosses_on_the_package_limit_too() {
        // Bytes are nowhere near their limit; packages are at theirs.
        let svc = svc_with(block_config(1_000_000, 10), 10, 10);
        let rows = svc
            .status_for_user("alice", &["cargo".to_owned()])
            .await
            .unwrap();
        assert_eq!(
            rows[0].state,
            QuotaState::AtLimit,
            "either dimension reaching its limit is at-limit"
        );
    }

    /// RFC 0004 §4.2 asks *which* threshold was crossed, so the two dimensions
    /// carry their own verdicts and the row's is the worse of them.
    #[tokio::test]
    async fn status_for_user_reports_each_dimension_separately() {
        // Storage at 68% of its limit; versions at 82% of theirs.
        let svc = svc_with(block_config(1_000, 50), 680, 41);
        let row = svc
            .status_for_user("alice", &["cargo".to_owned()])
            .await
            .unwrap()
            .remove(0);

        assert_eq!(
            row.bytes_state,
            Some(QuotaState::Ok),
            "68% is not a warning"
        );
        assert_eq!(row.packages_state, Some(QuotaState::Warning));
        assert_eq!(
            row.state,
            QuotaState::Warning,
            "the row takes the worse of its dimensions"
        );
    }

    #[tokio::test]
    async fn status_for_user_leaves_an_unlimited_dimension_stateless() {
        let mut configs = HashMap::new();
        configs.insert(
            "cargo".to_owned(),
            RegistryQuotaConfig {
                max_storage_bytes_per_user: Some(1_000),
                max_packages_per_user: None,
                warn_threshold: 0.8,
                enforcement: QuotaEnforcement::Block,
            },
        );
        let svc = QuotaService::new(MockQuotaRepo::new(900, 99), configs);
        let row = svc
            .status_for_user("alice", &["cargo".to_owned()])
            .await
            .unwrap()
            .remove(0);

        assert_eq!(row.bytes_state, Some(QuotaState::Warning));
        assert_eq!(
            row.packages_state, None,
            "a dimension with no limit has nothing to be near"
        );
        assert_eq!(row.state, QuotaState::Warning);
    }
}
