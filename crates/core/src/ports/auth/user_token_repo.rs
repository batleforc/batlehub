use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::Role;
use crate::error::CoreError;

pub struct UserToken {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub role: Role,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait UserTokenRepository: Send + Sync {
    async fn create_token(
        &self,
        id: Uuid,
        user_id: &str,
        name: &str,
        token_hash: &str,
        role: Role,
        expires_at: DateTime<Utc>,
    ) -> Result<UserToken, CoreError>;

    /// Look up an active (non-expired, non-revoked) token by its SHA-256 hash.
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<UserToken>, CoreError>;

    async fn list_for_user(&self, user_id: &str) -> Result<Vec<UserToken>, CoreError>;

    /// Soft-delete a token. Returns true if a row was revoked.
    async fn revoke(&self, id: Uuid, user_id: &str) -> Result<bool, CoreError>;
}
