pub mod in_memory;
pub use in_memory::InMemoryCacheStore;

#[cfg(feature = "db-postgres")]
pub mod postgres;
#[cfg(feature = "db-postgres")]
pub use postgres::PgCacheStore;

#[cfg(feature = "cache-redis")]
pub mod redis;
#[cfg(feature = "cache-redis")]
pub use redis::RedisCacheStore;
