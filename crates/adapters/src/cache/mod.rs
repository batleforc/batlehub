pub mod banner_memory;
pub use banner_memory::InMemoryBannerStore;
pub mod in_memory;
pub use in_memory::InMemoryCacheStore;

#[cfg(feature = "db-postgres")]
pub mod postgres;
#[cfg(feature = "db-postgres")]
pub use postgres::PgCacheStore;

#[cfg(feature = "cache-redis")]
pub mod banner_redis;
#[cfg(feature = "cache-redis")]
pub use banner_redis::RedisBannerStore;
#[cfg(feature = "cache-redis")]
pub mod redis;
#[cfg(feature = "cache-redis")]
pub use redis::RedisCacheStore;

#[cfg(feature = "cache-redis")]
pub mod warm_coordinator;
#[cfg(feature = "cache-redis")]
pub use warm_coordinator::RedisWarmCoordinator;
