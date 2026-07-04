use std::collections::HashMap;

use async_trait::async_trait;

use crate::entities::Identity;
use crate::error::CoreError;

/// Raw auth data extracted from an HTTP request before any provider processes it.
#[derive(Debug, Clone)]
pub struct RawAuthRequest {
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
}

/// Authenticates an HTTP request and returns the caller's identity.
///
/// Returns `Ok(None)` when this provider does not recognise the credentials
/// (the auth middleware will then try the next provider).
/// Returns `Ok(Some(identity))` on successful authentication.
/// Returns `Err` only on internal provider failures (network, crypto, etc.).
#[async_trait]
pub trait AuthProvider: Send + Sync {
    fn name(&self) -> &str;

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<Identity>, CoreError>;
}
