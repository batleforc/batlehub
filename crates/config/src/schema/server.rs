use std::net::IpAddr;

use ipnet::IpNet;
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
    /// CIDR ranges (or bare IPs) of the reverse proxies in front of BatleHub.
    /// `Forwarded` / `X-Forwarded-Host` / `X-Forwarded-Proto` / `X-Forwarded-For`
    /// are honoured only when the TCP peer falls inside one of these.
    ///
    /// ```toml
    /// [server]
    /// trusted_proxies = ["10.42.0.0/16", "192.168.1.10"]
    /// ```
    ///
    /// Three distinguishable states, which is why this is an `Option`:
    ///
    /// | Value    | Behaviour                                                        |
    /// |----------|------------------------------------------------------------------|
    /// | absent   | legacy permissive — forwarded host/scheme believed unconditionally, `X-Forwarded-For` ignored. A hard error once host-based routing is configured. |
    /// | `[]`     | forwarded headers ignored entirely; the `Host` header and the connection decide. |
    /// | `[nets]` | forwarded headers honoured only from peers inside those prefixes. |
    ///
    /// A bare address is accepted and treated as a `/32` (`/128` for IPv6), so
    /// every value that was valid for the deprecated
    /// `[ip_blocking].trusted_proxies` stays valid here.
    #[serde(default)]
    pub trusted_proxies: Option<Vec<String>>,
}

/// Parse `trusted_proxies` entries into CIDR prefixes.
///
/// A bare address is widened to its host prefix (`/32` for IPv4, `/128` for
/// IPv6) so the deprecated `[ip_blocking].trusted_proxies` exact-IP values keep
/// matching exactly what they matched before.
pub fn parse_trusted_proxies(entries: &[String]) -> Result<Vec<IpNet>, String> {
    entries
        .iter()
        .map(|entry| {
            let trimmed = entry.trim();
            trimmed
                .parse::<IpNet>()
                .or_else(|_| trimmed.parse::<IpAddr>().map(IpNet::from))
                .map_err(|_| {
                    format!(
                        "invalid trusted proxy entry '{entry}': expected an IP address \
                         (10.0.0.1) or a CIDR range (10.42.0.0/16)"
                    )
                })
        })
        .collect()
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
    /// Minimum number of idle connections the pool keeps warm.
    #[serde(default = "default_min_connections")]
    pub min_connections: u32,
    /// How long to wait for a connection to become available before erroring out.
    #[serde(default = "default_acquire_timeout_secs")]
    pub acquire_timeout_secs: u64,
}

pub(super) fn default_max_connections() -> u32 {
    10
}

pub(super) fn default_min_connections() -> u32 {
    1
}

pub(super) fn default_acquire_timeout_secs() -> u64 {
    30
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
