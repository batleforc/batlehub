use std::sync::Arc;

use batlehub_core::{entities::GlobalBanner, error::CoreError, ports::BannerPort};

/// Thin wrapper around a `BannerPort` implementation.
///
/// The active backend (in-memory / Redis / PostgreSQL) is selected at startup
/// based on the cache configuration and injected via `new()`.
pub struct BannerService {
    store: Arc<dyn BannerPort>,
}

impl BannerService {
    pub fn new(store: Arc<dyn BannerPort>) -> Self {
        Self { store }
    }

    pub async fn get(&self) -> Result<Option<GlobalBanner>, CoreError> {
        self.store.get().await
    }

    pub async fn set(&self, banner: GlobalBanner) -> Result<(), CoreError> {
        self.store.set(banner).await
    }

    pub async fn clear(&self) -> Result<(), CoreError> {
        self.store.clear().await
    }
}
