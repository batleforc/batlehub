use serde::Deserialize;

// ── Server ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Directory from which to serve the built SPA (optional).
    pub static_dir: Option<String>,
    /// Path to the `batlehub-cli` binary to serve via `GET /api/v1/cli/download`.
    /// When absent the endpoint returns 404.
    ///
    /// ```toml
    /// [server]
    /// cli_binary_path = "/usr/local/bin/batlehub-cli"
    /// ```
    #[serde(default)]
    pub cli_binary_path: Option<String>,
    /// Allowed CORS origins. When set, only the listed origins receive
    /// Access-Control-Allow-Origin headers. When absent, all origins are
    /// allowed (suitable for development; restrict in production).
    #[serde(default)]
    pub cors_allowed_origins: Option<Vec<String>>,
}

pub(super) fn default_host() -> String {
    "0.0.0.0".to_owned()
}

pub(super) fn default_port() -> u16 {
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

pub(super) fn default_max_connections() -> u32 {
    10
}

// ── Cache backend ─────────────────────────────────────────────────────────────

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

pub(super) fn default_cache_type() -> String {
    "memory".to_owned()
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            cache_type: default_cache_type(),
            url: None,
        }
    }
}

// ── OTel ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OtelConfig {
    /// OTLP endpoint, e.g. `http://localhost:4317`.
    pub endpoint: String,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

pub fn default_service_name() -> String {
    "batlehub".to_owned()
}
