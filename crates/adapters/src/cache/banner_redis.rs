use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;

use batlehub_core::{entities::GlobalBanner, error::CoreError, ports::BannerPort};

const BANNER_KEY: &str = "batlehub:system:banner";

/// Redis-backed banner store. Suitable for multi-instance (HA) deployments.
pub struct RedisBannerStore {
    conn: ConnectionManager,
}

impl RedisBannerStore {
    pub async fn new(url: &str) -> Result<Self, anyhow::Error> {
        let client = redis::Client::open(url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }
}

#[async_trait]
impl BannerPort for RedisBannerStore {
    async fn get(&self) -> Result<Option<GlobalBanner>, CoreError> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = conn
            .get(BANNER_KEY)
            .await
            .map_err(|e| CoreError::Cache(e.to_string()))?;
        match raw {
            None => Ok(None),
            Some(s) => {
                let banner: GlobalBanner = serde_json::from_str(&s)
                    .map_err(|e| CoreError::Cache(format!("banner deserialize: {e}")))?;
                Ok(Some(banner))
            }
        }
    }

    async fn set(&self, banner: GlobalBanner) -> Result<(), CoreError> {
        let mut conn = self.conn.clone();
        let json = serde_json::to_string(&banner)
            .map_err(|e| CoreError::Cache(format!("banner serialize: {e}")))?;
        conn.set::<_, _, ()>(BANNER_KEY, json)
            .await
            .map_err(|e| CoreError::Cache(e.to_string()))
    }

    async fn clear(&self) -> Result<(), CoreError> {
        let mut conn = self.conn.clone();
        conn.del::<_, ()>(BANNER_KEY)
            .await
            .map_err(|e| CoreError::Cache(e.to_string()))
    }
}
