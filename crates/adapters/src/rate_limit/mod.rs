use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub mod in_memory;
pub use in_memory::InMemoryRateLimitStore;

pub mod ip_block_in_memory;
pub use ip_block_in_memory::InMemoryIpBlockStore;

#[cfg(feature = "db-postgres")]
pub mod postgres;
#[cfg(feature = "db-postgres")]
pub use postgres::PgRateLimitStore;

#[cfg(feature = "db-postgres")]
pub mod ip_block_postgres;
#[cfg(feature = "db-postgres")]
pub use ip_block_postgres::PgIpBlockStore;

#[cfg(feature = "cache-redis")]
pub mod redis;
#[cfg(feature = "cache-redis")]
pub use redis::RedisRateLimitStore;

#[cfg(feature = "cache-redis")]
pub mod ip_block_redis;
#[cfg(feature = "cache-redis")]
pub use ip_block_redis::RedisIpBlockStore;
