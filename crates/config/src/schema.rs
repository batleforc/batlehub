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
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        for registry in &self.registries {
            if registry.name.is_empty() {
                bail!("registry is missing a 'name' field");
            }
            match registry.registry_type.as_str() {
                "github" | "cargo" | "npm" | "openvsx" | "goproxy" | "pypi" | "composer"
                | "vscode-marketplace" => {}
                other => bail!("unknown registry type: '{other}'"),
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

// Keep the old name as an alias so existing code compiles during migration.
#[allow(dead_code)]
pub type StorageConfig = StoragesConfig;

// ── Cache ─────────────────────────────────────────────────────────────────────

/// Selects the metadata cache backend.
///
/// In TOML:
/// ```toml
/// [cache]
/// type = "postgres"   # "memory" (default) | "postgres"
/// ```
#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    /// `"memory"` (default) uses an in-process HashMap; no persistence between restarts.
    /// `"postgres"` stores entries in the `metadata_cache` table; survives restarts and
    /// is shared across multiple server instances.
    #[serde(rename = "type", default = "default_cache_type")]
    pub cache_type: String,
}

fn default_cache_type() -> String {
    "memory".to_owned()
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { cache_type: default_cache_type() }
    }
}

// ── Registries ────────────────────────────────────────────────────────────────

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
    /// How to handle artifact caching: `"permanent"` (never re-fetch) or `"ttl"`.
    #[serde(default = "default_artifact_strategy")]
    pub artifact_strategy: String,
    /// When true (the default), serve stale metadata when upstream returns a transient
    /// error instead of propagating a 502. Allows cached artifacts to keep being served
    /// during upstream outages.
    #[serde(default = "default_serve_stale")]
    pub serve_stale: bool,
}

fn default_metadata_ttl() -> u64 {
    300
}

fn default_artifact_strategy() -> String {
    "permanent".to_owned()
}

fn default_serve_stale() -> bool {
    true
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self {
            metadata_ttl_secs: default_metadata_ttl(),
            artifact_strategy: default_artifact_strategy(),
            serve_stale: true,
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
