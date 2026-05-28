use std::collections::HashMap;

use anyhow::{bail, Result};
use serde::Deserialize;

// ── Top-level ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AppConfig {
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
        for registry in &self.registries {
            if registry.name.is_empty() {
                bail!("registry is missing a 'name' field");
            }
            match registry.registry_type.as_str() {
                "github" | "cargo" | "npm" | "openvsx" | "goproxy" | "pypi" | "composer"
                | "vscode-marketplace" | "maven" | "terraform" | "rubygems" => {}
                other => bail!("unknown registry type: '{other}'"),
            }
            if matches!(registry.mode, RegistryMode::Local | RegistryMode::Hybrid)
                && !matches!(
                    registry.registry_type.as_str(),
                    "cargo"
                        | "npm"
                        | "openvsx"
                        | "vscode-marketplace"
                        | "goproxy"
                        | "rubygems"
                        | "maven"
                        | "terraform"
                        | "composer"
                )
            {
                bail!(
                    "registry '{}': mode 'local'/'hybrid' is only supported for cargo, npm, openvsx, vscode-marketplace, goproxy, rubygems, maven, terraform, and composer registries",
                    registry.name
                );
            }
            if registry.mode == RegistryMode::Hybrid && registry.upstreams.is_empty() {
                bail!(
                    "registry '{}': hybrid mode requires at least one upstream URL",
                    registry.name
                );
            }
        }
        Ok(())
    }

    /// Apply environment variable overrides on top of the file-based config.
    ///
    /// Convention: `PROXY_CACHE__<SECTION>__<FIELD>` (double-underscore separator).
    ///
    /// Supported variables:
    /// | Variable                              | Field                        |
    /// |---------------------------------------|------------------------------|
    /// | `PROXY_CACHE__SERVER__HOST`           | `server.host`                |
    /// | `PROXY_CACHE__SERVER__PORT`           | `server.port`                |
    /// | `PROXY_CACHE__SERVER__STATIC_DIR`     | `server.static_dir`          |
    /// | `PROXY_CACHE__DATABASE__URL`          | `database.url`               |
    /// | `PROXY_CACHE__DATABASE__MAX_CONNECTIONS` | `database.max_connections` |
    /// | `PROXY_CACHE__STORAGE__PATH`          | `storage.path` (single filesystem backend only)  |
    /// | `PROXY_CACHE__STORAGE__BUCKET`        | `storage.bucket` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__REGION`        | `storage.region` (single S3 backend only)        |
    /// | `PROXY_CACHE__STORAGE__ENDPOINT_URL`  | `storage.endpoint_url` (single S3 backend only)  |
    /// | `PROXY_CACHE__OTEL__ENDPOINT`         | `otel.endpoint`              |
    /// | `PROXY_CACHE__OTEL__SERVICE_NAME`     | `otel.service_name`          |
    pub fn apply_env_overrides(&mut self) {
        let env = |key: &str| std::env::var(key).ok();

        // server
        if let Some(v) = env("PROXY_CACHE__SERVER__HOST") { self.server.host = v; }
        if let Some(v) = env("PROXY_CACHE__SERVER__PORT") {
            if let Ok(p) = v.parse() { self.server.port = p; }
        }
        if let Some(v) = env("PROXY_CACHE__SERVER__STATIC_DIR") { self.server.static_dir = Some(v); }

        // database
        if let Some(v) = env("PROXY_CACHE__DATABASE__URL") { self.database.url = v; }
        if let Some(v) = env("PROXY_CACHE__DATABASE__MAX_CONNECTIONS") {
            if let Ok(n) = v.parse() { self.database.max_connections = n; }
        }

        // storage env overrides (only supported for legacy single-backend config)
        if let StoragesConfig::Single(ref mut backend) = self.storage {
            if let Some(v) = env("PROXY_CACHE__STORAGE__PATH") {
                if let StorageBackendConfig::Filesystem(fs) = backend { fs.path = v; }
            }
            if let Some(v) = env("PROXY_CACHE__STORAGE__BUCKET") {
                if let StorageBackendConfig::S3(s3) = backend { s3.bucket = v; }
            }
            if let Some(v) = env("PROXY_CACHE__STORAGE__REGION") {
                if let StorageBackendConfig::S3(s3) = backend { s3.region = v; }
            }
            if let Some(v) = env("PROXY_CACHE__STORAGE__ENDPOINT_URL") {
                if let StorageBackendConfig::S3(s3) = backend { s3.endpoint_url = Some(v); }
            }
        }

        // otel — creates the section if not present in the file
        if let Some(v) = env("PROXY_CACHE__OTEL__ENDPOINT") {
            match &mut self.otel {
                Some(otel) => otel.endpoint = v,
                None => self.otel = Some(OtelConfig { endpoint: v, service_name: default_service_name() }),
            }
        }
        if let Some(v) = env("PROXY_CACHE__OTEL__SERVICE_NAME") {
            if let Some(otel) = &mut self.otel { otel.service_name = v; }
        }
    }
}

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Directory from which to serve the built SPA (optional).
    pub static_dir: Option<String>,
    /// Allowed CORS origins. When set, only the listed origins receive
    /// Access-Control-Allow-Origin headers. When absent, all origins are
    /// allowed (suitable for development; restrict in production).
    #[serde(default)]
    pub cors_allowed_origins: Option<Vec<String>>,
}

fn default_host() -> String {
    "0.0.0.0".to_owned()
}

fn default_port() -> u16 {
    8080
}

// ── Database ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(rename = "type")]
    pub db_type: String,
    pub url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

fn default_max_connections() -> u32 {
    10
}

// ── Auth ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum AuthConfig {
    Token(TokenAuthConfig),
    Oidc(OidcAuthConfig),
    Kubernetes(KubernetesAuthConfig),
}

#[derive(Debug, Deserialize)]
pub struct TokenAuthConfig {
    #[serde(default)]
    pub tokens: Vec<TokenEntry>,
}

#[derive(Debug, Deserialize)]
pub struct TokenEntry {
    pub value: String,
    pub role: String,
    pub user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OidcAuthConfig {
    /// Unique name for this provider instance.
    /// Used as the prefix for unmapped groups: e.g. `name = "oidc1"` → group `"oidc1:team-a"`.
    /// Also used as `"*:team-a"` wildcard target in `[registries.rbac.groups]`.
    /// Defaults to `"oidc"`.
    #[serde(default = "default_oidc_name")]
    pub name: String,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    /// Redirect URI registered with the OIDC provider.
    /// Required for the browser-based SSO login flow.
    /// Example: `"https://batlehub.example.com/api/v1/auth/oidc/callback"`.
    pub redirect_uri: Option<String>,
    /// Base URL of the SPA frontend.  After a successful OIDC callback the
    /// browser is redirected to `{frontend_url}/?oidc_access_token=...`.
    /// Defaults to `""` (same origin as the backend — correct for production).
    /// In development set this to `"http://localhost:5173"`.
    #[serde(default)]
    pub frontend_url: String,
    /// OAuth2 scopes to request.  Defaults to `["openid", "profile", "email"]`.
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>,
    /// JWT claim used as `user_id` (default: `"sub"`).
    #[serde(default = "default_sub")]
    pub user_id_claim: String,
    /// JWT claim to inspect for role mapping (default: `"role"`).
    /// The claim value may be a string or an array of strings; the highest
    /// matching role in `role_mappings` wins.
    #[serde(default = "default_role_claim")]
    pub role_claim: String,
    /// Maps JWT claim values → proxy role names (`"admin"`, `"user"`).
    /// Claim values not present here default to the `anonymous` role.
    #[serde(default)]
    pub role_mappings: std::collections::HashMap<String, String>,
}

fn default_oidc_name() -> String {
    "oidc".to_owned()
}

fn default_oidc_scopes() -> Vec<String> {
    vec!["openid".to_owned(), "profile".to_owned(), "email".to_owned()]
}

fn default_sub() -> String {
    "sub".to_owned()
}

fn default_role_claim() -> String {
    "role".to_owned()
}

#[derive(Debug, Deserialize)]
pub struct KubernetesAuthConfig {
    /// Unique name for this provider instance.
    /// Used as the prefix for unmapped groups: e.g. `name = "k8s-prod"` → group `"k8s-prod:team-a"`.
    /// Defaults to `"kubernetes"`.
    #[serde(default = "default_kubernetes_name")]
    pub name: String,
    /// Kubernetes API server URL.
    /// Defaults to `https://<KUBERNETES_SERVICE_HOST>:<KUBERNETES_SERVICE_PORT>`
    /// (the env vars injected by Kubernetes for in-cluster use).
    pub api_server: Option<String>,
    /// Path to the CA certificate PEM file for the Kubernetes API server.
    /// Defaults to `/var/run/secrets/kubernetes.io/serviceaccount/ca.crt`.
    pub ca_cert_path: Option<String>,
    /// Path to the batlehub's own service account token used to authenticate
    /// TokenReview API calls.
    /// Defaults to `/var/run/secrets/kubernetes.io/serviceaccount/token`.
    pub token_path: Option<String>,
    /// Audiences passed to the TokenReview API for bound-token validation.
    /// Defaults to `["batlehub"]` when empty.
    #[serde(default)]
    pub audiences: Vec<String>,
    /// Maps Kubernetes usernames or group names to proxy roles.
    ///
    /// Kubernetes populates:
    /// - username: `"system:serviceaccount:<namespace>:<name>"`
    /// - groups:   `["system:serviceaccounts", "system:serviceaccounts:<namespace>", ...]`
    ///
    /// Values not listed here default to the `anonymous` role.
    /// When a token matches multiple keys, the highest role wins.
    #[serde(default)]
    pub role_mappings: std::collections::HashMap<String, String>,
}

fn default_kubernetes_name() -> String {
    "kubernetes".to_owned()
}

// ── Storage ───────────────────────────────────────────────────────────────────

/// Accepts both the legacy single-backend form and the new multi-backend form.
///
/// Legacy (single backend, backwards-compatible):
/// ```toml
/// [storage]
/// type = "filesystem"
/// path = "./tmp/cache"
/// ```
///
/// Multi-backend:
/// ```toml
/// [storage]
/// default = "primary"
///
/// [[storage.backends]]
/// name = "primary"
/// type = "filesystem"
/// path = "./tmp/cache"
///
/// [[storage.backends]]
/// name = "rustfs"
/// type = "s3"
/// bucket = "artifacts"
/// region = "us-east-1"
/// endpoint_url = "http://localhost:9900"
/// force_path_style = true
/// ```
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum StoragesConfig {
    /// Legacy single backend (no `default` or `backends` keys).
    Single(StorageBackendConfig),
    /// New multi-backend with explicit default selection.
    Multi(MultiStorageConfig),
}

#[derive(Debug, Deserialize)]
pub struct MultiStorageConfig {
    pub default: String,
    pub backends: Vec<NamedStorageConfig>,
}

#[derive(Debug, Deserialize)]
pub struct NamedStorageConfig {
    pub name: String,
    #[serde(flatten)]
    pub config: StorageBackendConfig,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StorageBackendConfig {
    Filesystem(FilesystemStorageConfig),
    S3(S3StorageConfig),
}

#[derive(Debug, Deserialize)]
pub struct FilesystemStorageConfig {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct S3StorageConfig {
    pub bucket: String,
    pub region: String,
    pub prefix: Option<String>,
    pub endpoint_url: Option<String>,
    /// Use path-style URLs (required for RustFS, MinIO, and other S3-compatible stores).
    pub force_path_style: Option<bool>,
}

// ── Cache ─────────────────────────────────────────────────────────────────────

/// Selects the metadata cache backend.
///
/// In TOML:
/// ```toml
/// [cache]
/// type = "postgres"   # "memory" (default) | "postgres" | "redis"
///
/// # Required when type = "redis":
/// url = "redis://localhost:6379"
/// ```
#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    /// `"memory"` (default) uses an in-process HashMap; no persistence between restarts.
    /// `"postgres"` stores entries in the `metadata_cache` table; survives restarts and
    /// is shared across multiple server instances.
    /// `"redis"` stores entries in Redis; survives restarts and is shared across instances.
    #[serde(rename = "type", default = "default_cache_type")]
    pub cache_type: String,
    /// Connection URL for the Redis cache backend (required when `type = "redis"`).
    /// Format: `redis://[:<password>@]<host>[:<port>][/<db>]`
    /// or `rediss://...` for TLS.
    pub url: Option<String>,
}

fn default_cache_type() -> String {
    "memory".to_owned()
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { cache_type: default_cache_type(), url: None }
    }
}

// ── Registries ────────────────────────────────────────────────────────────────

/// Controls whether a registry acts as a caching proxy, a private authoritative
/// registry, or both.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RegistryMode {
    /// Forward all requests to upstream registries and cache responses.
    #[default]
    Proxy,
    /// BatleHub is the authoritative source; no upstream is consulted.
    Local,
    /// Check local publications first; fall back to upstream if not found.
    Hybrid,
}

#[derive(Debug, Deserialize)]
pub struct RegistryConfig {
    #[serde(rename = "type")]
    pub registry_type: String,
    pub name: String,
    /// Upstream URLs tried in order; if a registry returns 404 the next one is tried.
    /// When empty the adapter's built-in default (e.g. registry.npmjs.org) is used.
    #[serde(default)]
    pub upstreams: Vec<String>,
    /// Cargo only: URL of the sparse crate index.
    /// Defaults to `https://index.crates.io` when the upstream is crates.io.
    /// Set this for self-hosted registries (e.g. Gitea/Forgejo package feeds).
    #[serde(default)]
    pub index_url: Option<String>,
    #[serde(default)]
    pub cache: CachePolicy,
    #[serde(default)]
    pub rbac: RbacConfig,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
    /// Name of the storage backend to use for this registry's artifacts.
    /// Must match one of the backend names in `[[storage.backends]]`.
    /// When absent, the default backend is used.
    #[serde(default)]
    pub storage: Option<String>,
    /// When `true` the registry acts as a pure firewall: rules are evaluated but
    /// artifacts are never cached. Requests that pass rules are streamed directly
    /// from upstream with nothing written to storage.
    #[serde(default)]
    pub firewall_only: bool,
    /// Credentials to send on every upstream request for this registry.
    #[serde(default)]
    pub upstream_auth: Option<UpstreamAuthConfig>,
    /// TLS settings for upstream connections (e.g. custom CA certificate).
    #[serde(default)]
    pub tls: Option<UpstreamTlsConfig>,
    /// Controls proxy vs. local vs. hybrid behaviour for this registry.
    #[serde(default)]
    pub mode: RegistryMode,
    /// Optional publish quota enforced on local/hybrid registries.
    #[serde(default)]
    pub quota: Option<QuotaConfig>,
    /// Optional per-user request rate limit for this registry.
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    /// Optional versioning policy enforced at publish time (local/hybrid mode only).
    #[serde(default)]
    pub versioning: Option<VersioningPolicy>,
    /// Optional artifact signing configuration (local/hybrid mode only).
    #[serde(default)]
    pub signing: Option<SigningConfig>,
    /// Optional beta-channel configuration (local/hybrid mode only).
    /// When enabled, pre-release versions are only visible to registered beta-channel members.
    #[serde(default)]
    pub beta_channel: Option<BetaChannelConfig>,
}

// ── Versioning policy ─────────────────────────────────────────────────────────

/// Per-registry versioning policy enforced at publish time.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VersioningPolicy {
    /// Reject publish if the version string is not a valid semver (e.g. `1.2.3`, `1.0.0-beta.1`).
    #[serde(default)]
    pub enforce_semver: bool,
    /// Reject publish if the semver pre-release component is non-empty (e.g. `-alpha`, `-beta.1`).
    /// Only effective when `enforce_semver` is also `true`.
    #[serde(default = "default_true")]
    pub allow_prerelease: bool,
    /// Reject publish if the version string does not match this regex.
    #[serde(default)]
    pub version_pattern: Option<String>,
}

fn default_true() -> bool { true }

// ── Artifact signing ──────────────────────────────────────────────────────────

/// Per-registry artifact signing configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SigningConfig {
    /// When `true`, reject publish requests that do not include an `X-Artifact-Signature` header.
    #[serde(default)]
    pub required: bool,
    /// Accepted signature types (e.g. `["pgp", "ed25519"]`).
    /// When empty, any type (or no type) is accepted.
    #[serde(default)]
    pub allowed_types: Vec<String>,
}

// ── Quota management ──────────────────────────────────────────────────────────

/// How to enforce quota violations.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QuotaEnforcement {
    /// Reject the publish request with HTTP 429 when the quota is exceeded.
    #[default]
    Block,
    /// Allow the publish but include a warning header in the response.
    Warn,
}

/// Per-registry publish quotas for local/hybrid mode.
///
/// Example TOML:
/// ```toml
/// [registries.quota]
/// max_storage_bytes_per_user = 1_073_741_824   # 1 GiB
/// max_packages_per_user      = 100
/// warn_threshold_pct         = 80
/// enforcement                = "block"
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct QuotaConfig {
    /// Maximum cumulative bytes a single user may publish to this registry.
    pub max_storage_bytes_per_user: Option<u64>,
    /// Maximum number of distinct package versions a single user may publish.
    pub max_packages_per_user: Option<u32>,
    /// Emit a quota-warning response header when usage exceeds this percentage
    /// of the limit. Defaults to 80.
    #[serde(default = "default_warn_pct")]
    pub warn_threshold_pct: u8,
    /// Whether to hard-block or just warn on quota overrun.
    #[serde(default)]
    pub enforcement: QuotaEnforcement,
}

fn default_warn_pct() -> u8 { 80 }

// ── Beta channel ──────────────────────────────────────────────────────────────

/// Per-registry beta-channel configuration (local/hybrid mode only).
///
/// When `enabled` is `true`, pre-release versions (semver versions with a
/// non-empty pre-release component, e.g. `1.0.0-beta.1`) are hidden from users
/// who are not registered as beta-channel members. Non-members receive 404 on
/// both index listings and artifact downloads for pre-release versions.
///
/// ```toml
/// [registries.beta_channel]
/// enabled = true
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BetaChannelConfig {
    /// Enable pre-release gating for this registry.
    #[serde(default)]
    pub enabled: bool,
}

// ── IP-based blocking ─────────────────────────────────────────────────────────

/// Global fail2ban-style IP blocking configuration.
///
/// The middleware counts "violation events" (responses with status codes in
/// `trigger_on_status`) per client IP. When the count exceeds
/// `violation_threshold` within `violation_window_secs`, the IP is automatically
/// blocked for `ban_duration_secs`. Blocked IPs receive HTTP 403 immediately,
/// before auth or rate-limit checks.
///
/// ```toml
/// [ip_blocking]
/// enabled               = true
/// violation_threshold   = 10
/// violation_window_secs = 300
/// ban_duration_secs     = 3600
/// trigger_on_status     = [429, 401]
/// # List the exact IPs of your reverse proxies so that X-Forwarded-For is
/// # trusted only from those hosts. Leave empty (the default) to always use
/// # the TCP peer address — required when the server is exposed directly.
/// trusted_proxies       = ["10.0.0.1", "10.0.0.2"]
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct IpBlockingConfig {
    /// Enable IP-based blocking.
    #[serde(default)]
    pub enabled: bool,
    /// Number of violations in the window before auto-blocking the IP.
    #[serde(default = "default_violation_threshold")]
    pub violation_threshold: u32,
    /// Length of the violation counting window in seconds.
    #[serde(default = "default_violation_window")]
    pub violation_window_secs: u32,
    /// How long to keep a blocked IP banned, in seconds.
    #[serde(default = "default_ban_duration")]
    pub ban_duration_secs: u64,
    /// HTTP status codes that increment the violation counter for the source IP.
    #[serde(default = "default_trigger_on_status")]
    pub trigger_on_status: Vec<u16>,
    /// IPs of trusted reverse proxies. When the TCP peer address matches one of
    /// these, the first entry of `X-Forwarded-For` is used as the client IP.
    /// When empty (the default), the TCP peer address is always used and
    /// `X-Forwarded-For` is ignored — preventing spoofed-header bypass.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
}

impl Default for IpBlockingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            violation_threshold: default_violation_threshold(),
            violation_window_secs: default_violation_window(),
            ban_duration_secs: default_ban_duration(),
            trigger_on_status: default_trigger_on_status(),
            trusted_proxies: Vec::new(),
        }
    }
}

fn default_violation_threshold() -> u32 { 10 }
fn default_violation_window() -> u32 { 300 }
fn default_ban_duration() -> u64 { 3600 }
fn default_trigger_on_status() -> Vec<u16> { vec![429, 401] }

// ── Rate limiting ─────────────────────────────────────────────────────────────

/// How to enforce rate-limit violations.
#[derive(Debug, Deserialize, Default, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RateLimitEnforcement {
    /// Reject requests that exceed the rate limit with HTTP 429.
    #[default]
    Block,
    /// Allow the request but include a warning header in the response.
    Warn,
}

/// Per-registry request rate limiting.
///
/// Example TOML:
/// ```toml
/// [registries.rate_limit]
/// requests_per_window = 100
/// window_secs         = 60
/// enforcement         = "block"
///
/// [[registries.rate_limit.groups]]
/// name                = "ci-bots"
/// requests_per_window = 5000   # shared pool for all ci-bots members combined
/// window_secs         = 60
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of requests a single user (or IP for anonymous) may make within the window.
    pub requests_per_window: u32,
    /// Length of the sliding window in seconds.
    pub window_secs: u32,
    /// Whether to hard-block (429) or just warn on rate-limit overrun.
    #[serde(default)]
    pub enforcement: RateLimitEnforcement,
    /// Optional per-group rate limits. Each entry defines a shared request pool for all
    /// members of the named group. A user's request is checked against both their personal
    /// bucket and every group bucket they belong to; all must have tokens available.
    #[serde(default)]
    pub groups: Vec<GroupRateLimitConfig>,
}

/// A shared request pool for all members of a named group.
///
/// The `name` is matched against the namespaced group strings in `Identity.groups`
/// (e.g. `"oidc:ci-bots"` or `"*:ci-bots"` for a wildcard provider prefix).
///
/// Example TOML:
/// ```toml
/// [[registries.rate_limit.groups]]
/// name                = "oidc:ci-bots"
/// requests_per_window = 5000
/// window_secs         = 60
/// enforcement         = "block"   # optional; inherits parent enforcement when omitted
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct GroupRateLimitConfig {
    /// Group name to match against `Identity.groups` (exact string match).
    pub name: String,
    /// Maximum requests the entire group may collectively make within the window.
    pub requests_per_window: u32,
    /// Length of the sliding window in seconds.
    pub window_secs: u32,
    /// Override enforcement for this group. Inherits the parent `enforcement` when absent.
    pub enforcement: Option<RateLimitEnforcement>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UpstreamAuthConfig {
    Bearer(BearerAuthConfig),
    Basic(BasicAuthConfig),
    Header(HeaderAuthConfig),
}

#[derive(Debug, Deserialize)]
pub struct BearerAuthConfig {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct BasicAuthConfig {
    pub username: String,
    pub password: String,
}

/// Sends a single custom HTTP header on every upstream request.
/// Useful for registries that use `X-API-Key` or similar schemes.
#[derive(Debug, Deserialize)]
pub struct HeaderAuthConfig {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpstreamTlsConfig {
    /// Path to a PEM-encoded CA certificate to trust for this registry's upstream.
    pub ca_cert_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CachePolicy {
    /// TTL for metadata (version lists, release info) in seconds.
    #[serde(default = "default_metadata_ttl")]
    pub metadata_ttl_secs: u64,
    /// When true (the default), serve stale metadata when upstream returns a transient
    /// error instead of propagating a 502. Allows cached artifacts to keep being served
    /// during upstream outages.
    #[serde(default = "default_serve_stale")]
    pub serve_stale: bool,
    /// Evict artifacts older than this many seconds. `null` means never expire by age.
    #[serde(default)]
    pub artifact_ttl_secs: Option<u64>,
    /// Evict artifacts not accessed for this many days. `null` means never expire by idle time.
    #[serde(default)]
    pub idle_days: Option<u64>,
    /// Storage size cap in bytes. When exceeded, the least-recently-used artifacts are evicted
    /// until usage falls below this threshold. `null` means no size cap.
    #[serde(default)]
    pub max_size_bytes: Option<u64>,
    /// Keep only the N most-recently-cached versions per (registry, package). Older versions
    /// are evicted when a new one is stored. `null` means keep all versions.
    #[serde(default)]
    pub keep_latest_n: Option<usize>,
    /// Packages to pre-fetch on startup and via the `/warm` admin endpoint.
    /// Each entry is either a bare package name (`"lodash"`) or a pinned version
    /// (`"lodash@4.17.21"`). Bare names warm the latest `warm_latest_n` versions.
    #[serde(default)]
    pub warm_packages: Vec<String>,
    /// Number of most-recent versions to pre-warm per package (default: 1 = latest only).
    #[serde(default = "default_warm_latest_n")]
    pub warm_latest_n: usize,
    /// Maximum number of concurrent artifact downloads during a warming run (default: 2).
    #[serde(default = "default_warm_concurrency")]
    pub warm_concurrency: usize,
}

fn default_metadata_ttl() -> u64 {
    300
}

fn default_serve_stale() -> bool {
    true
}

fn default_warm_latest_n() -> usize {
    1
}

fn default_warm_concurrency() -> usize {
    2
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            metadata_ttl_secs: default_metadata_ttl(),
            serve_stale: true,
            artifact_ttl_secs: None,
            idle_days: None,
            max_size_bytes: None,
            keep_latest_n: None,
            warm_packages: vec![],
            warm_latest_n: default_warm_latest_n(),
            warm_concurrency: default_warm_concurrency(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct RbacConfig {
    #[serde(default)]
    pub anonymous: Vec<String>,
    #[serde(default)]
    pub user: Vec<String>,
    #[serde(default)]
    pub admin: Vec<String>,
    /// Dynamic groups from external identity providers (e.g. Authentik).
    /// Maps group name → list of permitted resource types for this registry.
    #[serde(default)]
    pub groups: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleConfig {
    ReleaseAgeGate(ReleaseAgeGateConfig),
    RequireSignedRelease(RequireSignedReleaseConfig),
    DenyLatest(DenyLatestConfig),
}

#[derive(Debug, Deserialize)]
pub struct ReleaseAgeGateConfig {
    /// Minimum age in seconds before a release is downloadable.
    #[serde(default = "default_min_age")]
    pub min_age_secs: u64,
    /// Roles that may bypass the age gate (e.g. `["admin"]`).
    #[serde(default)]
    pub bypass_roles: Vec<String>,
}

fn default_min_age() -> u64 {
    3600
}

#[derive(Debug, Deserialize)]
pub struct RequireSignedReleaseConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct DenyLatestConfig {
    /// Roles that may bypass the restriction (e.g. `["admin"]`).
    #[serde(default)]
    pub bypass_roles: Vec<String>,
}

// ── OTEL ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OtelConfig {
    /// OTLP endpoint, e.g. `http://localhost:4317`.
    pub endpoint: String,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

fn default_service_name() -> String {
    "batlehub".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_policy_defaults() {
        let p: CachePolicy = toml::from_str("").unwrap();
        assert_eq!(p.metadata_ttl_secs, 300);
        assert!(p.serve_stale);
        assert!(p.artifact_ttl_secs.is_none());
        assert!(p.idle_days.is_none());
        assert!(p.max_size_bytes.is_none());
        assert!(p.keep_latest_n.is_none());
    }

    #[test]
    fn cache_policy_full_config() {
        let raw = r#"
            metadata_ttl_secs = 60
            serve_stale = false
            artifact_ttl_secs = 3600
            idle_days = 30
            max_size_bytes = 10000000
            keep_latest_n = 5
        "#;
        let p: CachePolicy = toml::from_str(raw).unwrap();
        assert_eq!(p.metadata_ttl_secs, 60);
        assert!(!p.serve_stale);
        assert_eq!(p.artifact_ttl_secs, Some(3600));
        assert_eq!(p.idle_days, Some(30));
        assert_eq!(p.max_size_bytes, Some(10_000_000));
        assert_eq!(p.keep_latest_n, Some(5));
    }

    #[test]
    fn cache_policy_partial_config_uses_defaults_for_unset_fields() {
        let raw = "artifact_ttl_secs = 7200";
        let p: CachePolicy = toml::from_str(raw).unwrap();
        assert_eq!(p.metadata_ttl_secs, 300, "metadata_ttl_secs should use default");
        assert!(p.serve_stale, "serve_stale should default to true");
        assert_eq!(p.artifact_ttl_secs, Some(7200));
        assert!(p.idle_days.is_none());
        assert!(p.max_size_bytes.is_none());
        assert!(p.keep_latest_n.is_none());
    }

    #[test]
    fn cache_policy_zero_keep_latest_n_is_valid() {
        let raw = "keep_latest_n = 1";
        let p: CachePolicy = toml::from_str(raw).unwrap();
        assert_eq!(p.keep_latest_n, Some(1));
    }

    #[test]
    fn cache_policy_default_impl_matches_toml_defaults() {
        let from_default = CachePolicy::default();
        let from_toml: CachePolicy = toml::from_str("").unwrap();
        assert_eq!(from_default.metadata_ttl_secs, from_toml.metadata_ttl_secs);
        assert_eq!(from_default.serve_stale, from_toml.serve_stale);
        assert_eq!(from_default.artifact_ttl_secs, from_toml.artifact_ttl_secs);
        assert_eq!(from_default.idle_days, from_toml.idle_days);
        assert_eq!(from_default.max_size_bytes, from_toml.max_size_bytes);
        assert_eq!(from_default.keep_latest_n, from_toml.keep_latest_n);
    }
}
