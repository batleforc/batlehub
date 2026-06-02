use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use batlehub_core::{entities::GlobalBanner, error::CoreError, ports::BannerPort};

/// In-memory banner store. State is lost on process restart; suitable for
/// single-instance deployments or when persistence is not required.
pub struct InMemoryBannerStore {
    current: Arc<RwLock<Option<GlobalBanner>>>,
}

impl Default for InMemoryBannerStore {
    fn default() -> Self {
        Self {
            current: Arc::new(RwLock::new(None)),
        }
    }
}

impl InMemoryBannerStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BannerPort for InMemoryBannerStore {
    async fn get(&self) -> Result<Option<GlobalBanner>, CoreError> {
        Ok(self.current.read().await.clone())
    }

    async fn set(&self, banner: GlobalBanner) -> Result<(), CoreError> {
        *self.current.write().await = Some(banner);
        Ok(())
    }

    async fn clear(&self) -> Result<(), CoreError> {
        *self.current.write().await = None;
        Ok(())
    }
}
