pub mod cache;
pub mod in_memory;
pub mod migrations;
pub mod notification;
pub mod rate_limit;
pub mod sbom;

#[cfg(feature = "auth-token")]
pub mod auth;

#[cfg(feature = "storage-fs")]
pub mod storage;

pub mod registry;

/// Debian APT / RPM / Arch pacman repository hosting: package parsing, index
/// generation, and Ed25519 OpenPGP signing. Gated on the deb/rpm/pacman registry
/// features.
#[cfg(any(
    feature = "registry-deb",
    feature = "registry-rpm",
    feature = "registry-pacman"
))]
pub mod repo;

#[cfg(feature = "vuln-scan")]
pub mod vulnerability;

#[cfg(feature = "db-postgres")]
pub mod db;

#[cfg(feature = "local-registry")]
pub mod local_registry;
