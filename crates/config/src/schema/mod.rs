pub mod auth;
pub mod network;
pub mod notifications;
pub mod registry;
pub mod rules;
pub mod server;
pub mod storage;

pub use auth::{
    ActionsGroupRule, ActionsOidcAuthConfig, AuthConfig, Condition, ConditionMatchType,
    KubernetesAuthConfig, OidcAuthConfig, RuleMatch, TokenAuthConfig, TokenEntry,
};
pub use network::{
    BasicAuthConfig, BearerAuthConfig, GroupRateLimitConfig, HeaderAuthConfig, IpBlockingConfig,
    RateLimitConfig, RateLimitEnforcement, UpstreamAuthConfig, UpstreamProxyConfig,
    UpstreamTlsConfig,
};
pub use notifications::{
    EmailChannelConfig, InboundWebhookConfig, NotificationChannelConfig, NotificationsConfig,
    SlackChannelConfig, TeamsChannelConfig, WebhookChannelConfig,
};
pub use registry::{
    default_true, BetaChannelConfig, CachePolicy, FeatureFlagsConfig, IntegrityConfig, QuotaConfig,
    QuotaEnforcement, RegistryConfig, RegistryMode, RepoSigningConfig, SbomConfig, SigningConfig,
    VersioningPolicy,
};
pub use rules::{
    CveGateConfig, DenyLatestConfig, ExploreRbacConfig, RbacConfig, ReleaseAgeGateConfig,
    RequireSignedReleaseConfig, RuleConfig, TrustedPublisherConfig, VersionGateConfig,
};
pub use server::{default_service_name, CacheConfig, DatabaseConfig, OtelConfig, ServerConfig};
pub use storage::{
    FilesystemStorageConfig, MultiStorageConfig, NamedStorageConfig, S3StorageConfig,
    StorageBackendConfig, StoragesConfig,
};

use anyhow::{bail, Result};
use serde::Deserialize;

// ── Top-level ─────────────────────────────────────────────────────────────────

/// The current config schema version this binary understands. Bump this only
/// for changes that would silently break an existing config file if applied
/// unchanged (removing/renaming a field, changing a default's meaning) — see
/// "Config versioning" in `docs/configuration.md`.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    /// Config schema version. Optional; absent is treated as
    /// [`CURRENT_CONFIG_VERSION`] so every existing config file keeps working
    /// unchanged. An explicit value newer than this binary supports is
    /// rejected at startup rather than silently misbehaving.
    #[serde(default)]
    pub config_version: Option<u32>,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: Vec<AuthConfig>,
    pub storage: StoragesConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub registries: Vec<RegistryConfig>,
    #[serde(default)]
    pub otel: Option<OtelConfig>,
    #[serde(default)]
    pub limits: LimitsConfig,
    /// Optional global IP-based blocking (fail2ban) configuration.
    #[serde(default)]
    pub ip_blocking: Option<IpBlockingConfig>,
    /// Optional webhook and notification configuration.
    #[serde(default)]
    pub notifications: Option<NotificationsConfig>,
    /// Global HTTP/SOCKS proxy applied to all registry upstreams that do not
    /// define their own `[registries.proxy]` section.
    ///
    /// Can be overridden at runtime via `PROXY_CACHE__PROXY__URL` (and related
    /// variables) without changing the config file.
    #[serde(default)]
    pub proxy: Option<UpstreamProxyConfig>,
    /// Optional periodic re-check of cached SBOMs against the OSV vulnerability
    /// database. When absent or `enabled = false`, no background scan runs.
    #[serde(default)]
    pub vulnerability_scan: Option<VulnerabilityScanConfig>,
}

// ── Vulnerability scan ──────────────────────────────────────────────────────────

fn default_vuln_interval_secs() -> u64 {
    86_400
}

fn default_vuln_batch_size() -> usize {
    100
}

/// Periodic SBOM-vs-CVE re-check configuration.
///
/// ```toml
/// [vulnerability_scan]
/// enabled       = true
/// interval_secs = 86400          # daily
/// osv_api_url   = "https://api.osv.dev"
/// batch_size    = 100
/// ```
#[derive(Debug, Deserialize)]
pub struct VulnerabilityScanConfig {
    /// Enable the periodic background scan.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between scan runs. Defaults to one day.
    #[serde(default = "default_vuln_interval_secs")]
    pub interval_secs: u64,
    /// Base URL of the OSV API. Defaults to `https://api.osv.dev` when absent.
    #[serde(default)]
    pub osv_api_url: Option<String>,
    /// Number of SBOMs processed per page. Defaults to 100.
    #[serde(default = "default_vuln_batch_size")]
    pub batch_size: usize,
}

// ── Limits ────────────────────────────────────────────────────────────────────

/// Upload size limits.
///
/// ```toml
/// [limits]
/// max_artifact_size_bytes = 524288000  # 500 MiB
/// ```
#[derive(Debug, Deserialize, Default)]
pub struct LimitsConfig {
    /// Maximum artifact size for proxy downloads and local publishes.
    /// Defaults to 500 MiB when absent.
    pub max_artifact_size_bytes: Option<u64>,
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if let Some(v) = self.config_version {
            if v > CURRENT_CONFIG_VERSION {
                bail!(
                    "config_version {v} is newer than this binary supports (max {CURRENT_CONFIG_VERSION}); \
                     upgrade batlehub-server, or lower config_version if you intended to target an \
                     older schema"
                );
            }
        }
        for registry in &self.registries {
            if registry.name.is_empty() {
                bail!("registry is missing a 'name' field");
            }
            let kind: batlehub_core::entities::RegistryKind =
                registry.registry_type.parse().map_err(anyhow::Error::msg)?;
            if matches!(registry.mode, RegistryMode::Local | RegistryMode::Hybrid)
                && !kind.supports_local_mode()
            {
                bail!(
                    "registry '{}': mode 'local'/'hybrid' is not supported for {} registries (no local publish model)",
                    registry.name,
                    kind
                );
            }
            if registry.mode == RegistryMode::Hybrid && registry.upstreams.is_empty() {
                bail!(
                    "registry '{}': hybrid mode requires at least one upstream URL",
                    registry.name
                );
            }
            // deb/rpm have no universal default upstream, so proxy mode (which would
            // otherwise fall back to an unreachable placeholder) also requires an
            // explicit upstream. Caught at startup instead of every fetch failing.
            if registry.mode == RegistryMode::Proxy
                && registry.upstreams.is_empty()
                && kind.requires_explicit_upstream_in_proxy_mode()
            {
                bail!(
                    "registry '{}': {} proxy mode requires at least one upstream URL (no default upstream exists)",
                    registry.name,
                    kind
                );
            }
        }
        Ok(())
    }

    /// Apply environment variable overrides on top of the file-based config.
    ///
    /// **Preferred approach for secrets:** use `${VAR_NAME}` placeholders directly
    /// inside the TOML file — they are expanded before parsing, so they work for
    /// any field, including `client_secret`, upstream auth `token`/`password`/`value`,
    /// and any other string field.  See the docs for details.
    ///
    /// This method handles a fixed set of named overrides for non-secret top-level
    /// fields as a convenience.  Convention: `PROXY_CACHE__<SECTION>__<FIELD>`
    /// (double-underscore separator).
    ///
    /// Supported variables:
    /// | Variable                              | Field                        |
    /// |---------------------------------------|------------------------------|
    /// | `PROXY_CACHE__SERVER__HOST`           | `server.host`                |
    /// | `PROXY_CACHE__SERVER__PORT`           | `server.port`                |
    /// | `PROXY_CACHE__SERVER__STATIC_DIR`     | `server.static_dir`          |
    /// | `PROXY_CACHE__DATABASE__URL`          | `database.url`               |
    /// | `PROXY_CACHE__DATABASE__MAX_CONNECTIONS` | `database.max_connections` |
    /// | `PROXY_CACHE__DATABASE__MIN_CONNECTIONS` | `database.min_connections` |
    /// | `PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS` | `database.acquire_timeout_secs` |
    /// | `PROXY_CACHE__STORAGE__PATH`          | `storage.path` (single filesystem backend only)  |
    /// | `PROXY_CACHE__STORAGE__BUCKET`        | `storage.bucket` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__REGION`        | `storage.region` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__ENDPOINT_URL`  | `storage.endpoint_url` (single S3 backend only)  |
    /// | `PROXY_CACHE__OTEL__ENDPOINT`         | `otel.endpoint`              |
    /// | `PROXY_CACHE__OTEL__SERVICE_NAME`     | `otel.service_name`          |
    pub fn apply_env_overrides(&mut self) {
        let env = |key: &str| std::env::var(key).ok();

        if let Some(v) = env("PROXY_CACHE__SERVER__HOST") {
            self.server.host = v;
        }
        if let Some(v) = env("PROXY_CACHE__SERVER__PORT") {
            if let Ok(p) = v.parse() {
                self.server.port = p;
            }
        }
        if let Some(v) = env("PROXY_CACHE__SERVER__STATIC_DIR") {
            self.server.static_dir = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__URL") {
            self.database.url = v;
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__MAX_CONNECTIONS") {
            if let Ok(n) = v.parse() {
                self.database.max_connections = n;
            }
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__MIN_CONNECTIONS") {
            if let Ok(n) = v.parse() {
                self.database.min_connections = n;
            }
        }
        if let Some(v) = env("PROXY_CACHE__DATABASE__ACQUIRE_TIMEOUT_SECS") {
            if let Ok(n) = v.parse() {
                self.database.acquire_timeout_secs = n;
            }
        }

        apply_storage_env_overrides(&mut self.storage, &env);
        apply_otel_env_overrides(&mut self.otel, &env);
        apply_proxy_env_overrides(&mut self.proxy, &env);
    }
}

fn apply_storage_env_overrides(storage: &mut StoragesConfig, env: &dyn Fn(&str) -> Option<String>) {
    let StoragesConfig::Single(ref mut backend) = storage else {
        return;
    };
    match backend {
        StorageBackendConfig::Filesystem(fs) => apply_filesystem_env_overrides(fs, env),
        StorageBackendConfig::S3(s3) => apply_s3_env_overrides(s3, env),
    }
}

fn apply_filesystem_env_overrides(
    fs: &mut FilesystemStorageConfig,
    env: &dyn Fn(&str) -> Option<String>,
) {
    if let Some(v) = env("PROXY_CACHE__STORAGE__PATH") {
        fs.path = v;
    }
}

fn apply_s3_env_overrides(s3: &mut S3StorageConfig, env: &dyn Fn(&str) -> Option<String>) {
    if let Some(v) = env("PROXY_CACHE__STORAGE__BUCKET") {
        s3.bucket = v;
    }
    if let Some(v) = env("PROXY_CACHE__STORAGE__REGION") {
        s3.region = v;
    }
    if let Some(v) = env("PROXY_CACHE__STORAGE__ENDPOINT_URL") {
        s3.endpoint_url = Some(v);
    }
}

fn apply_otel_env_overrides(otel: &mut Option<OtelConfig>, env: &dyn Fn(&str) -> Option<String>) {
    if let Some(v) = env("PROXY_CACHE__OTEL__ENDPOINT") {
        match otel {
            Some(o) => o.endpoint = v,
            None => {
                *otel = Some(OtelConfig {
                    endpoint: v,
                    service_name: server::default_service_name(),
                })
            }
        }
    }
    if let Some(v) = env("PROXY_CACHE__OTEL__SERVICE_NAME") {
        if let Some(o) = otel {
            o.service_name = v;
        }
    }
}

fn apply_proxy_env_overrides(
    proxy: &mut Option<UpstreamProxyConfig>,
    env: &dyn Fn(&str) -> Option<String>,
) {
    if let Some(v) = env("PROXY_CACHE__PROXY__URL") {
        match proxy {
            Some(p) => p.url = v,
            None => {
                *proxy = Some(UpstreamProxyConfig {
                    url: v,
                    username: env("PROXY_CACHE__PROXY__USERNAME"),
                    password: env("PROXY_CACHE__PROXY__PASSWORD"),
                    no_proxy: env("PROXY_CACHE__PROXY__NO_PROXY"),
                })
            }
        }
    }
    if let Some(p) = proxy {
        if let Some(v) = env("PROXY_CACHE__PROXY__USERNAME") {
            p.username = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__PROXY__PASSWORD") {
            p.password = Some(v);
        }
        if let Some(v) = env("PROXY_CACHE__PROXY__NO_PROXY") {
            p.no_proxy = Some(v);
        }
    }
}

#[cfg(test)]
mod tests;
