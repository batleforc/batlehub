use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use batlehub_core::{
    entities::Role,
    error::CoreError,
    ports::{UserToken, UserTokenRepository},
};

/// A [`UserTokenRepository`] that rejects token creation and returns empty
/// results for all lookups.
///
/// Appropriate for tests that authenticate via [`StaticTokenAuthProvider`]
/// (static tokens in config) and do not exercise the user-generated-token flow.
///
/// [`StaticTokenAuthProvider`]: crate::auth::StaticTokenAuthProvider
#[derive(Debug, Default)]
pub struct NullUserTokenRepository;

impl NullUserTokenRepository {
    pub fn arc() -> Arc<dyn UserTokenRepository> {
        Arc::new(Self)
    }
}

#[async_trait]
impl UserTokenRepository for NullUserTokenRepository {
    async fn create_token(
        &self,
        _id: Uuid,
        _user_id: &str,
        _name: &str,
        _token_hash: &str,
        _role: Role,
        _expires_at: DateTime<Utc>,
    ) -> Result<UserToken, CoreError> {
        Err(CoreError::Database(
            "NullUserTokenRepository does not support token creation".into(),
        ))
    }

    async fn find_by_hash(&self, _token_hash: &str) -> Result<Option<UserToken>, CoreError> {
        Ok(None)
    }

    async fn list_for_user(&self, _user_id: &str) -> Result<Vec<UserToken>, CoreError> {
        Ok(vec![])
    }

    async fn revoke(&self, _id: Uuid, _user_id: &str) -> Result<bool, CoreError> {
        Ok(false)
    }
}
