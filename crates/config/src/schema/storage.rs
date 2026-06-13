use serde::Deserialize;

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
