use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    entities::Identity,
    error::CoreError,
    ports::{QuotaRepository, QuotaUsage},
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

        let usage = self.repo.get_usage(&user_id, registry).await?;

        let new_bytes = usage.bytes_published.saturating_add(bytes);
        let new_count = usage.packages_count.saturating_add(1);

        // Check storage limit
        if let Some(max_bytes) = config.max_storage_bytes_per_user {
            if new_bytes > max_bytes {
                let msg = format!(
                    "storage quota exceeded for registry '{registry}': \
                     {new_bytes} bytes used, limit is {max_bytes}"
                );
                if config.enforcement == QuotaEnforcement::Block {
                    return Err(CoreError::QuotaExceeded(msg));
                }
                // Warn mode: log and continue
                tracing::warn!("{msg}");
            }
        }

        // Check package count limit
        if let Some(max_pkgs) = config.max_packages_per_user {
            if new_count > max_pkgs {
                let msg = format!(
                    "package quota exceeded for registry '{registry}': \
                     {new_count} packages, limit is {max_pkgs}"
                );
                if config.enforcement == QuotaEnforcement::Block {
                    return Err(CoreError::QuotaExceeded(msg));
                }
                tracing::warn!("{msg}");
            }
        }

        // Record the publish
        self.repo.record_publish(&user_id, registry, bytes).await?;

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
