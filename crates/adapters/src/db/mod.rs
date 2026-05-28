#[cfg(feature = "db-postgres")]
pub mod artifact_meta;

#[cfg(feature = "db-postgres")]
pub mod beta_channel;

#[cfg(feature = "db-postgres")]
pub mod ownership;

#[cfg(feature = "db-postgres")]
pub mod postgres;

#[cfg(feature = "db-postgres")]
pub mod quota;

#[cfg(feature = "db-postgres")]
pub mod team_namespace;

#[cfg(feature = "db-postgres")]
pub mod user_tokens;

#[cfg(feature = "db-postgres")]
pub use artifact_meta::PgArtifactMetaRepository;

#[cfg(feature = "db-postgres")]
pub use beta_channel::PgBetaChannelStore;

#[cfg(feature = "db-postgres")]
pub use ownership::PgOwnershipStore;

#[cfg(feature = "db-postgres")]
pub use postgres::PgPackageRepository;

#[cfg(feature = "db-postgres")]
pub use quota::PgQuotaRepository;

#[cfg(feature = "db-postgres")]
pub use team_namespace::PgTeamNamespaceStore;
