/// In-memory implementations of all core port traits.
///
/// These are suitable for tests, integration harnesses, and any scenario
/// that does not need persistence. All types are always compiled (no feature
/// gates) and are thread-safe via `tokio::sync::RwLock`.
///
/// Re-exported at the crate root as `batlehub_adapters::in_memory::*`.
pub mod artifact_meta;
pub mod beta_channel;
pub mod ownership;
pub mod package_repo;
pub mod quota;
pub mod sbom;
pub mod storage;
pub mod team_namespace;
pub mod user_tokens;
pub mod vulnerability;

pub use artifact_meta::NoopArtifactMetaRepository;
pub use beta_channel::InMemoryBetaChannelStore;
pub use ownership::InMemoryOwnershipStore;
pub use package_repo::InMemoryPackageRepository;
pub use quota::InMemoryQuotaRepository;
pub use sbom::{InMemorySbomRepository, NoopSbomRepository};
pub use storage::InMemoryStorageBackend;
pub use team_namespace::InMemoryTeamNamespaceStore;
pub use user_tokens::NullUserTokenRepository;
pub use vulnerability::InMemoryVulnerabilityRepository;
