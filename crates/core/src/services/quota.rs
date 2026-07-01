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

/// Result of a quota check. Returned from `QuotaService::check_and_record_publish`.
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
    pub async fn check_and_record_publish(
        &self,
        identity: &Identity,
        registry: &str,
        bytes: u64,
    ) -> Result<QuotaCheck, CoreError> {
        let config = match self.configs.get(registry) {
            Some(c) => c,
            None => {
                // No quota configured for this registry — pass through.
                return Ok(QuotaCheck::default());
            }
        };

        let user_id = match &identity.user_id {
            Some(id) => id.clone(),
            None => {
                // Anonymous users: enforce if limits exist, otherwise pass.
                if config.max_storage_bytes_per_user.is_some()
                    || config.max_packages_per_user.is_some()
                {
                    return Err(CoreError::QuotaExceeded(
                        "anonymous users cannot publish to quota-gated registries".into(),
                    ));
                }
                return Ok(QuotaCheck::default());
            }
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
                let msg = if config
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
                };
                return Err(CoreError::QuotaExceeded(msg));
            }
        };

        // In Warn mode the publish always records even past the limit; log it
        // server-side the same way the old check-then-write path did, since
        // nothing rejected the request to make the operator aware otherwise.
        if config.enforcement == QuotaEnforcement::Warn {
            if let Some(max) = config.max_storage_bytes_per_user {
                if new_bytes > max {
                    tracing::warn!(
                        "storage quota exceeded for registry '{registry}': \
                         {new_bytes} bytes used, limit is {max}"
                    );
                }
            }
            if let Some(max) = config.max_packages_per_user {
                if new_count > max {
                    tracing::warn!(
                        "package quota exceeded for registry '{registry}': \
                         {new_count} packages, limit is {max}"
                    );
                }
            }
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
}

fn is_warning(used: u64, limit: Option<u64>, threshold: f64) -> bool {
    match limit {
        Some(max) if max > 0 => used as f64 / max as f64 >= threshold,
        _ => false,
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
            .check_and_record_publish(&Identity::anonymous(), "cargo", 100)
            .await;
        assert!(matches!(result, Err(CoreError::QuotaExceeded(_))));
    }

    #[tokio::test]
    async fn byte_limit_blocks_when_enforcement_is_block() {
        let svc = svc_with(block_config(1000, 100), 900, 1);
        let result = svc
            .check_and_record_publish(&user("alice"), "cargo", 200)
            .await;
        assert!(matches!(result, Err(CoreError::QuotaExceeded(_))));
    }

    #[tokio::test]
    async fn no_quota_config_passes_through() {
        let svc = QuotaService::new(MockQuotaRepo::new(0, 0), HashMap::new());
        let check = svc
            .check_and_record_publish(&user("alice"), "cargo", 100)
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
            .check_and_record_publish(&user("alice"), "cargo", 200)
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
}
