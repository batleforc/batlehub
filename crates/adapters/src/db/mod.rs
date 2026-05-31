/// Extension trait that converts a `sqlx::Error` into `CoreError::Database` with a single
/// `.db_err()` suffix, eliminating the repeated
/// `.map_err(|e| CoreError::Database(e.to_string()))` boilerplate across all adapters.
#[cfg(feature = "db-postgres")]
pub(crate) trait DbResultExt<T> {
    fn db_err(self) -> Result<T, batlehub_core::error::CoreError>;
}

#[cfg(feature = "db-postgres")]
impl<T> DbResultExt<T> for Result<T, sqlx::Error> {
    fn db_err(self) -> Result<T, batlehub_core::error::CoreError> {
        self.map_err(|e| batlehub_core::error::CoreError::Database(e.to_string()))
    }
}

#[cfg(feature = "db-postgres")]
pub mod artifact_meta;

#[cfg(feature = "db-postgres")]
pub mod banner_pg;

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
pub use banner_pg::PgBannerStore;

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
