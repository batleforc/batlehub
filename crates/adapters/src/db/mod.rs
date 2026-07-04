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

/// Time a DB call and record it under `batlehub_db_query_duration_seconds`.
///
/// This is applied at a small, deliberately partial set of hot-path call
/// sites (the highest-traffic write and a cheap health-check read), not
/// exhaustively across every `sqlx::query` call in the codebase — there is no
/// existing chokepoint that all ~120 call sites funnel through, so full
/// coverage would require a much larger refactor. Extend coverage here
/// incrementally as more paths turn out to matter. `pub` (not `pub(crate)`) so
/// `crates/web`'s `/healthz` handler can share it instead of re-implementing
/// the same start/await/record sequence.
#[cfg(feature = "db-postgres")]
pub async fn timed_query<T>(
    query_name: &'static str,
    fut: impl std::future::Future<Output = T>,
) -> T {
    let start = std::time::Instant::now();
    let result = fut.await;
    metrics::histogram!("batlehub_db_query_duration_seconds", "query" => query_name)
        .record(start.elapsed().as_secs_f64());
    result
}

#[cfg(feature = "db-postgres")]
pub mod artifact_meta;

#[cfg(feature = "db-postgres")]
pub mod sbom;

#[cfg(feature = "db-postgres")]
pub mod banner;

#[cfg(feature = "db-postgres")]
pub mod beta_channel;

#[cfg(feature = "db-postgres")]
pub mod config_change;

#[cfg(feature = "db-postgres")]
pub mod ownership;

#[cfg(feature = "db-postgres")]
pub mod storage_admin;

#[cfg(feature = "db-postgres")]
pub mod postgres;

#[cfg(feature = "db-postgres")]
pub mod quota;

#[cfg(feature = "db-postgres")]
pub mod team_namespace;

#[cfg(feature = "db-postgres")]
pub mod user_block;

#[cfg(feature = "db-postgres")]
pub mod user_tokens;

#[cfg(feature = "db-postgres")]
pub mod vulnerability;

#[cfg(feature = "db-postgres")]
pub use artifact_meta::PgArtifactMetaRepository;

#[cfg(feature = "db-postgres")]
pub use banner::PgBannerStore;

#[cfg(feature = "db-postgres")]
pub use beta_channel::PgBetaChannelStore;

#[cfg(feature = "db-postgres")]
pub use config_change::PgConfigChangeRepository;

#[cfg(feature = "db-postgres")]
pub use ownership::PgOwnershipStore;

#[cfg(feature = "db-postgres")]
pub use storage_admin::PgStorageAdminRepository;

#[cfg(feature = "db-postgres")]
pub use postgres::PgPackageRepository;

#[cfg(feature = "db-postgres")]
pub use quota::PgQuotaRepository;

#[cfg(feature = "db-postgres")]
pub use sbom::PgSbomRepository;

#[cfg(feature = "db-postgres")]
pub use team_namespace::PgTeamNamespaceStore;

#[cfg(feature = "db-postgres")]
pub use user_block::{InMemoryUserBlockRepository, PgUserBlockRepository};

#[cfg(feature = "db-postgres")]
pub use vulnerability::PgVulnerabilityRepository;
