use async_trait::async_trait;

use crate::{entities::Identity, error::CoreError};

/// An entry granting a user or group access to pre-release versions in a registry.
pub struct BetaChannelEntry {
    pub principal_type: String, // "user" | "group"
    pub principal_id: String,
    pub granted_by: Option<String>,
}

/// Port for managing per-registry beta-channel membership.
///
/// When a beta channel is enabled for a registry, only members can see and download
/// pre-release versions (semver versions with a non-empty pre-release component).
/// Non-members receive stable versions only and get 404 on pre-release artifact downloads.
#[async_trait]
pub trait BetaChannelPort: Send + Sync {
    /// Returns `true` if the identity is a beta-channel member for the given registry.
    /// Anonymous identities always return `false`.
    async fn is_member(&self, registry: &str, identity: &Identity) -> Result<bool, CoreError>;

    /// Grant a user or group beta-channel access.
    async fn add_member(&self, registry: &str, entry: BetaChannelEntry) -> Result<(), CoreError>;

    /// Revoke beta-channel access.
    async fn remove_member(
        &self,
        registry: &str,
        principal_type: &str,
        principal_id: &str,
    ) -> Result<(), CoreError>;

    /// List all beta-channel members for a registry.
    async fn list_members(
        &self,
        registry: &str,
    ) -> Result<Vec<BetaChannelEntry>, CoreError>;
}
