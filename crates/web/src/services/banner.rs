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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use batlehub_adapters::cache::InMemoryBannerStore;
    use batlehub_core::entities::{BannerLevel, GlobalBanner};
    use chrono::Utc;

    use super::BannerService;

    fn make_svc() -> BannerService {
        BannerService::new(Arc::new(InMemoryBannerStore::new()))
    }

    fn banner(msg: &str) -> GlobalBanner {
        GlobalBanner {
            message: msg.to_owned(),
            level: BannerLevel::Info,
            set_at: Utc::now(),
            set_by: "test".to_owned(),
        }
    }

    #[tokio::test]
    async fn get_returns_none_initially() {
        let svc = make_svc();
        assert!(svc.get().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_and_get_round_trips() {
        let svc = make_svc();
        svc.set(banner("hello")).await.unwrap();
        let result = svc.get().await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().message, "hello");
    }

    #[tokio::test]
    async fn clear_removes_banner() {
        let svc = make_svc();
        svc.set(banner("temporary")).await.unwrap();
        svc.clear().await.unwrap();
        assert!(svc.get().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_overwrites_existing_banner() {
        let svc = make_svc();
        svc.set(banner("first")).await.unwrap();
        svc.set(banner("second")).await.unwrap();
        assert_eq!(svc.get().await.unwrap().unwrap().message, "second");
    }
}
