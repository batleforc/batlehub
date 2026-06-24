use std::sync::Arc;

use anyhow::{Context, Result};

use batlehub_adapters::cache::InMemoryBannerStore;
#[cfg(feature = "cache-redis")]
use batlehub_adapters::cache::RedisBannerStore;
#[cfg(feature = "cache-redis")]
use batlehub_adapters::cache::RedisCacheStore;
use batlehub_adapters::cache::{InMemoryCacheStore, PgCacheStore};
use batlehub_adapters::db::PgBannerStore;
use batlehub_adapters::notification::PgNotificationStore;
use batlehub_adapters::rate_limit::{
    InMemoryIpBlockStore, InMemoryRateLimitStore, PgIpBlockStore, PgRateLimitStore,
};
#[cfg(feature = "cache-redis")]
use batlehub_adapters::rate_limit::{RedisIpBlockStore, RedisRateLimitStore};
use batlehub_adapters::sbom::HttpSbomFetcher;
use batlehub_config::schema::AppConfig;
use batlehub_core::ports::{
    BannerPort, CacheStore, IpBlockStore, NotificationPort, RateLimitStore, SbomRepository,
};
use batlehub_core::services::SbomService;

pub(super) const DEFAULT_REDIS_URL: &str = "redis://127.0.0.1:6379";

/// How far back `prune_expired` reaches on each sweep. Generously larger than
/// any realistic rate-limit window so a stale row is never pruned mid-window.
const RATE_LIMIT_PRUNE_RETENTION_SECS: u64 = 24 * 60 * 60;
const RATE_LIMIT_PRUNE_INTERVAL_SECS: u64 = 60;

/// Periodically delete expired `rate_limit_counters` rows in the background,
/// instead of pruning inline on every `increment()` call. Mirrors the detached
/// `tokio::spawn` + `tokio::time::interval` pattern used by
/// `watcher::spawn_periodic_vuln_scan`.
fn spawn_rate_limit_prune(store: Arc<PgRateLimitStore>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(
            RATE_LIMIT_PRUNE_INTERVAL_SECS,
        ));
        loop {
            ticker.tick().await;
            if let Err(e) = store.prune_expired(RATE_LIMIT_PRUNE_RETENTION_SECS).await {
                tracing::warn!(error = %e, "rate-limit: periodic prune failed");
            }
        }
    });
}

pub(super) async fn create_cache_store(
    config: &AppConfig,
    pool: sqlx::PgPool,
) -> Result<Arc<dyn CacheStore>> {
    let store: Arc<dyn CacheStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("metadata cache: postgres");
            Arc::new(PgCacheStore::new(pool))
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config.cache.url.as_deref().unwrap_or(DEFAULT_REDIS_URL);
                tracing::info!(url, "metadata cache: redis");
                Arc::new(
                    RedisCacheStore::new(url)
                        .await
                        .context("connecting to Redis cache")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!(
                    "compiled without cache-redis feature; falling back to in-memory cache"
                );
                Arc::new(InMemoryCacheStore::new())
            }
        }
        other => {
            if other != "memory" {
                tracing::warn!(cache_type = %other, "unknown cache type, falling back to memory");
            } else {
                tracing::info!("metadata cache: memory");
            }
            Arc::new(InMemoryCacheStore::new())
        }
    };
    Ok(store)
}

pub(super) async fn create_rate_limit_store(
    config: &AppConfig,
    pool: sqlx::PgPool,
) -> Result<Arc<dyn RateLimitStore>> {
    let store: Arc<dyn RateLimitStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("rate limit store: postgres");
            let store = Arc::new(PgRateLimitStore::new(pool));
            spawn_rate_limit_prune(Arc::clone(&store));
            store
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config.cache.url.as_deref().unwrap_or(DEFAULT_REDIS_URL);
                tracing::info!(url, "rate limit store: redis");
                Arc::new(
                    RedisRateLimitStore::new(url)
                        .await
                        .context("connecting to Redis rate limit store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!(
                    "compiled without cache-redis feature; falling back to in-memory rate limit store"
                );
                Arc::new(InMemoryRateLimitStore::new())
            }
        }
        other => {
            if other != "memory" {
                tracing::warn!(
                    cache_type = %other,
                    "unknown cache type for rate limit store, falling back to memory"
                );
            }
            Arc::new(InMemoryRateLimitStore::new())
        }
    };
    Ok(store)
}

pub(super) async fn create_ip_block_store(
    config: &AppConfig,
    pool: sqlx::PgPool,
) -> Result<Arc<dyn IpBlockStore>> {
    let store: Arc<dyn IpBlockStore> = match config.cache.cache_type.as_str() {
        "postgres" => {
            tracing::info!("ip block store: postgres");
            Arc::new(PgIpBlockStore::new(pool))
        }
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config.cache.url.as_deref().unwrap_or(DEFAULT_REDIS_URL);
                tracing::info!(url, "ip block store: redis");
                Arc::new(
                    RedisIpBlockStore::new(url)
                        .await
                        .context("connecting to Redis ip block store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            {
                tracing::warn!(
                    "compiled without cache-redis feature; falling back to in-memory ip block store"
                );
                Arc::new(InMemoryIpBlockStore::new())
            }
        }
        _ => Arc::new(InMemoryIpBlockStore::new()),
    };
    Ok(store)
}

pub(super) async fn create_banner_store(
    config: &AppConfig,
    pool: sqlx::PgPool,
) -> Result<Arc<dyn BannerPort>> {
    let store: Arc<dyn BannerPort> = match config.cache.cache_type.as_str() {
        "postgres" => Arc::new(PgBannerStore::new(pool)),
        "redis" => {
            #[cfg(feature = "cache-redis")]
            {
                let url = config.cache.url.as_deref().unwrap_or(DEFAULT_REDIS_URL);
                Arc::new(
                    RedisBannerStore::new(url)
                        .await
                        .context("connecting to Redis banner store")?,
                )
            }
            #[cfg(not(feature = "cache-redis"))]
            Arc::new(InMemoryBannerStore::new())
        }
        _ => Arc::new(InMemoryBannerStore::new()),
    };
    Ok(store)
}

pub(super) fn create_notification_store(pool: sqlx::PgPool) -> Arc<dyn NotificationPort> {
    Arc::new(PgNotificationStore::new(pool))
}

pub(super) fn build_notification_service(
    notification_store: Arc<dyn NotificationPort>,
    notifications_config: &Option<batlehub_config::schema::NotificationsConfig>,
) -> Option<Arc<batlehub_web::services::NotificationService>> {
    let explicitly_disabled = notifications_config
        .as_ref()
        .map(|nc| !nc.enabled)
        .unwrap_or(false);
    if explicitly_disabled {
        return None;
    }
    let effective = notifications_config.as_ref().cloned().unwrap_or_default();
    Some(Arc::new(batlehub_web::services::NotificationService::new(
        notification_store,
        &effective,
    )))
}

pub(super) fn build_sbom_service(pool: sqlx::PgPool) -> Result<Arc<SbomService>> {
    use batlehub_adapters::db::PgSbomRepository;

    let sbom_repo: Arc<dyn SbomRepository> = Arc::new(PgSbomRepository::new(pool));
    #[cfg(feature = "sbom")]
    let sbom_extractor: Option<Arc<dyn batlehub_core::ports::SbomExtractor>> =
        Some(Arc::new(batlehub_adapters::sbom::ArchiveSbomExtractor));
    #[cfg(not(feature = "sbom"))]
    let sbom_extractor: Option<Arc<dyn batlehub_core::ports::SbomExtractor>> = None;

    let sbom_http = reqwest::Client::builder()
        .user_agent("batlehub/sbom")
        .build()
        .context("building SBOM HTTP client")?;
    Ok(Arc::new(SbomService::new(
        sbom_repo,
        sbom_extractor,
        Some(Arc::new(HttpSbomFetcher::new(sbom_http))),
    )))
}
