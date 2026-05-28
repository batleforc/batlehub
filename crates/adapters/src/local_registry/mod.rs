pub mod in_memory;
pub mod postgres;

pub use in_memory::InMemoryLocalRegistry;
pub use postgres::PostgresLocalRegistry;
