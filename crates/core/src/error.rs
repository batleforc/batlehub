use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Unknown registry: {0}")]
    UnknownRegistry(String),

    #[error("Package not found: {0}")]
    NotFound(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Registry error: {0}")]
    Registry(String),

    #[error("Auth error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("Integrity check failed: {0}")]
    IntegrityFailure(String),

    #[error("Quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("Invalid version: {0}")]
    InvalidVersion(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Invalid configuration: {0}")]
    Config(String),

    /// The operation is not part of this registry type's protocol — a capability
    /// probe, not a failure. Callers that have a fallback should match on it
    /// rather than propagate it.
    #[error("Not supported: {0}")]
    NotSupported(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
