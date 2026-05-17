use std::sync::Arc;

use async_trait::async_trait;
use rand::RngCore;
use sha2::{Digest, Sha256};

use proxy_cache_core::{
    entities::Identity,
    error::CoreError,
    ports::{AuthProvider, RawAuthRequest, UserTokenRepository},
};

pub fn generate_token() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let raw = hex::encode(bytes);
    let hash = hash_token(&raw);
    (raw, hash)
}

pub fn hash_token(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}

pub struct UserTokenAuthProvider {
    repo: Arc<dyn UserTokenRepository>,
}

impl UserTokenAuthProvider {
    pub fn new(repo: Arc<dyn UserTokenRepository>) -> Self {
        Self { repo }
    }
}

#[async_trait]
impl AuthProvider for UserTokenAuthProvider {
    fn name(&self) -> &str {
        "user-token"
    }

    async fn authenticate(&self, req: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
        let Some(header) = req.headers.get("authorization") else {
            return Ok(None);
        };

        let Some(raw) = header.strip_prefix("Bearer ") else {
            return Ok(None);
        };

        // Fast path: OIDC JWTs contain dots; our hex tokens never do.
        if raw.contains('.') {
            return Ok(None);
        }

        let hash = hash_token(raw);
        match self.repo.find_by_hash(&hash).await? {
            None => Ok(None),
            Some(tok) => Ok(Some(Identity {
                user_id: Some(tok.user_id),
                role: tok.role,
                auth_provider: Some("user-token".to_owned()),
            })),
        }
    }
}
