use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::entities::Role;
use crate::error::CoreError;

/// Who a personal access token belongs to.
///
/// A `user_id` on its own is not an identity: it is whatever string the
/// authenticating provider chose — an OIDC `sub`, a Kubernetes service account
/// username, a value an operator typed into a static `[[auth.tokens]]` entry.
/// Two providers can hand back the same string for entirely different people, so
/// the provider name has to travel with it. Listing and revocation used to match
/// on `user_id` alone, which meant any identity carrying the same string could
/// enumerate and destroy another principal's tokens.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenOwner {
    /// The `name` of the `[[auth]]` provider that authenticated the caller.
    pub provider: String,
    pub user_id: String,
}

impl TokenOwner {
    pub fn new(provider: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            user_id: user_id.into(),
        }
    }
}

pub struct UserToken {
    pub id: Uuid,
    pub user_id: String,
    /// Provider that authenticated the session which created this token.
    pub provider: String,
    pub name: String,
    pub role: Role,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// When the token was last presented, or `None` if not since this was
    /// recorded. Not the same as "never used" for a token that predates the
    /// column.
    pub last_used_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait UserTokenRepository: Send + Sync {
    async fn create_token(
        &self,
        id: Uuid,
        owner: &TokenOwner,
        name: &str,
        token_hash: &str,
        role: Role,
        expires_at: DateTime<Utc>,
    ) -> Result<UserToken, CoreError>;

    /// Look up an active (non-expired, non-revoked) token by its SHA-256 hash.
    async fn find_by_hash(&self, token_hash: &str) -> Result<Option<UserToken>, CoreError>;

    async fn list_for_user(&self, owner: &TokenOwner) -> Result<Vec<UserToken>, CoreError>;

    /// Record that `id` was just presented.
    ///
    /// Best-effort and off the critical path: a failure here must never turn a
    /// valid credential into a rejected one, so callers log and carry on.
    async fn touch_last_used(&self, id: Uuid) -> Result<(), CoreError>;

    /// Soft-delete a token. Returns true if a row was revoked.
    ///
    /// Scoped to `owner`, not just to the token id: a caller may only revoke
    /// what it owns, and ownership includes which provider it authenticated
    /// through.
    async fn revoke(&self, id: Uuid, owner: &TokenOwner) -> Result<bool, CoreError>;
}
