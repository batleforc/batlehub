/// In-memory implementations of all core port traits.
///
/// These are suitable for tests, integration harnesses, and any scenario
/// that does not need persistence. All types are always compiled (no feature
/// gates) and are thread-safe via `tokio::sync::RwLock`.
///
/// Re-exported at the crate root as `batlehub_adapters::in_memory::*`.
pub mod artifact_meta;
pub mod package_repo;
pub mod sbom;
pub mod vulnerability;

// ── Domain subfolders, mirroring `batlehub_core::ports`'s auth/governance/ops/storage split ──
// (registry-domain concerns stay flat above, as `package_repo`/`artifact_meta` already did
// before this split — there was no separate `registry/` port module to mirror there.)
pub mod auth;
pub mod governance;
pub mod ops;
pub mod storage;

pub use artifact_meta::NoopArtifactMetaRepository;
pub use auth::user_tokens::NullUserTokenRepository;
pub use governance::beta_channel::InMemoryBetaChannelStore;
pub use governance::ownership::InMemoryOwnershipStore;
pub use governance::team_namespace::InMemoryTeamNamespaceStore;
pub use ops::quota::InMemoryQuotaRepository;
pub use package_repo::InMemoryPackageRepository;
pub use sbom::{InMemorySbomRepository, NoopSbomRepository};
pub use storage::backend::InMemoryStorageBackend;
pub use vulnerability::InMemoryVulnerabilityRepository;
