use std::sync::Arc;

use async_trait::async_trait;
use rand::RngCore;
use sha2::{Digest, Sha256};

use batlehub_core::{
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
                groups: vec![],
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use async_trait::async_trait;
    use chrono::DateTime;
    use batlehub_core::{entities::Role, error::CoreError, ports::{RawAuthRequest, UserToken, UserTokenRepository}};
    use super::*;

    fn req(auth: &str) -> RawAuthRequest {
        RawAuthRequest {
            headers: HashMap::from([("authorization".to_owned(), auth.to_owned())]),
            query_params: HashMap::new(),
        }
    }

    fn no_auth_req() -> RawAuthRequest {
        RawAuthRequest { headers: HashMap::new(), query_params: HashMap::new() }
    }

    struct StubRepo(Option<UserToken>);

    fn stub_token() -> UserToken {
        UserToken {
            id: uuid::Uuid::new_v4(),
            user_id: "carol".to_owned(),
            name: "test-token".to_owned(),
            role: Role::User,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            revoked_at: None,
        }
    }

    #[async_trait]
    impl UserTokenRepository for StubRepo {
        async fn create_token(&self, _: uuid::Uuid, _: &str, _: &str, _: &str, _: Role, _: DateTime<chrono::Utc>) -> Result<UserToken, CoreError> {
            Ok(stub_token())
        }
        async fn find_by_hash(&self, _: &str) -> Result<Option<UserToken>, CoreError> {
            Ok(self.0.as_ref().map(|t| UserToken {
                id: t.id,
                user_id: t.user_id.clone(),
                name: t.name.clone(),
                role: t.role.clone(),
                created_at: t.created_at,
                expires_at: t.expires_at,
                revoked_at: t.revoked_at,
            }))
        }
        async fn list_for_user(&self, _: &str) -> Result<Vec<UserToken>, CoreError> { Ok(vec![]) }
        async fn revoke(&self, _: uuid::Uuid, _: &str) -> Result<bool, CoreError> { Ok(true) }
    }

    #[test]
    fn generate_token_produces_unique_values() {
        let (t1, h1) = generate_token();
        let (t2, h2) = generate_token();
        assert_ne!(t1, t2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_token_is_deterministic() {
        assert_eq!(hash_token("hello"), hash_token("hello"));
        assert_ne!(hash_token("hello"), hash_token("world"));
    }

    #[tokio::test]
    async fn no_auth_header_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        assert!(p.authenticate(&no_auth_req()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn non_bearer_header_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        assert!(p.authenticate(&req("Basic dXNlcjpwYXNz")).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn jwt_dot_in_token_short_circuits_without_repo_call() {
        // Repo would return a token, but the JWT detection must bypass it.
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let result = p.authenticate(&req("Bearer header.payload.sig")).await.unwrap();
        assert!(result.is_none(), "JWT tokens must not be looked up in the repo");
    }

    #[tokio::test]
    async fn valid_hex_token_returns_identity() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(Some(stub_token()))));
        let id = p.authenticate(&req("Bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")).await.unwrap().unwrap();
        assert_eq!(id.user_id.as_deref(), Some("carol"));
        assert_eq!(id.role, Role::User);
    }

    #[tokio::test]
    async fn unknown_token_returns_none() {
        let p = UserTokenAuthProvider::new(Arc::new(StubRepo(None)));
        let result = p.authenticate(&req("Bearer abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890")).await.unwrap();
        assert!(result.is_none());
    }
}
