use anyhow::{bail, Result};
use serde::Deserialize;

// ── Top-level ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    #[serde(default)]
    pub auth: Vec<AuthConfig>,
    pub storage: StorageConfig,
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
                "github" | "cargo" | "npm" | "pypi" | "composer" => {}
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
    /// | `PROXY_CACHE__STORAGE__PATH`          | `storage.path` (filesystem)  |
    /// | `PROXY_CACHE__STORAGE__BUCKET`        | `storage.bucket` (s3)        |
    /// | `PROXY_CACHE__STORAGE__REGION`        | `storage.region` (s3)        |
    /// | `PROXY_CACHE__STORAGE__ENDPOINT_URL`  | `storage.endpoint_url` (s3)  |
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

        // storage (type must come from the config file)
        if let Some(v) = env("PROXY_CACHE__STORAGE__PATH") {
            if let StorageConfig::Filesystem(fs) = &mut self.storage { fs.path = v; }
        }
        if let Some(v) = env("PROXY_CACHE__STORAGE__BUCKET") {
            if let StorageConfig::S3(s3) = &mut self.storage { s3.bucket = v; }
        }
        if let Some(v) = env("PROXY_CACHE__STORAGE__REGION") {
            if let StorageConfig::S3(s3) = &mut self.storage { s3.region = v; }
        }
        if let Some(v) = env("PROXY_CACHE__STORAGE__ENDPOINT_URL") {
            if let StorageConfig::S3(s3) = &mut self.storage { s3.endpoint_url = Some(v); }
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
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: Option<String>,
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

fn default_sub() -> String {
    "sub".to_owned()
}

fn default_role_claim() -> String {
    "role".to_owned()
}

#[derive(Debug, Deserialize)]
pub struct KubernetesAuthConfig {
    /// Kubernetes API server URL.
    /// Defaults to `https://<KUBERNETES_SERVICE_HOST>:<KUBERNETES_SERVICE_PORT>`
    /// (the env vars injected by Kubernetes for in-cluster use).
    pub api_server: Option<String>,
    /// Path to the CA certificate PEM file for the Kubernetes API server.
    /// Defaults to `/var/run/secrets/kubernetes.io/serviceaccount/ca.crt`.
    pub ca_cert_path: Option<String>,
    /// Path to the proxy-cache's own service account token used to authenticate
    /// TokenReview API calls.
    /// Defaults to `/var/run/secrets/kubernetes.io/serviceaccount/token`.
    pub token_path: Option<String>,
    /// Audiences passed to the TokenReview API for bound-token validation.
    /// Defaults to `["proxy-cache"]` when empty.
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

// ── Storage ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StorageConfig {
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
    #[serde(default)]
    pub cache: CachePolicy,
    #[serde(default)]
    pub rbac: RbacConfig,
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

#[derive(Debug, Deserialize, Default)]
pub struct CachePolicy {
    /// TTL for metadata (version lists, release info) in seconds.
    #[serde(default = "default_metadata_ttl")]
    pub metadata_ttl_secs: u64,
    /// How to handle artifact caching: `"permanent"` (never re-fetch) or `"ttl"`.
    #[serde(default = "default_artifact_strategy")]
    pub artifact_strategy: String,
}

fn default_metadata_ttl() -> u64 {
    300
}

fn default_artifact_strategy() -> String {
    "permanent".to_owned()
}

#[derive(Debug, Deserialize, Default)]
pub struct RbacConfig {
    #[serde(default)]
    pub anonymous: Vec<String>,
    #[serde(default)]
    pub user: Vec<String>,
    #[serde(default)]
    pub admin: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleConfig {
    ReleaseAgeGate(ReleaseAgeGateConfig),
    RequireSignedRelease(RequireSignedReleaseConfig),
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

// ── OTEL ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OtelConfig {
    /// OTLP endpoint, e.g. `http://localhost:4317`.
    pub endpoint: String,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

fn default_service_name() -> String {
    "proxy-cache".to_owned()
}
