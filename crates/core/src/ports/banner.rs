use async_trait::async_trait;

use crate::entities::GlobalBanner;
use crate::error::CoreError;

/// Storage port for the global admin banner.
///
/// Implementations: in-memory (single instance), Redis (multi-instance HA), PostgreSQL (multi-instance HA).
#[async_trait]
pub trait BannerPort: Send + Sync {
    async fn get(&self) -> Result<Option<GlobalBanner>, CoreError>;
    async fn set(&self, banner: GlobalBanner) -> Result<(), CoreError>;
    async fn clear(&self) -> Result<(), CoreError>;
}
