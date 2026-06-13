mod models;

mod client;
pub use client::ComposerRegistryClient;

mod impl_registry;

pub mod local;
pub use local::parse_composer_zip;

pub use models::ComposerPackageMeta;

// Re-export http_client so submodules can reach it via `super::http_client`.
use super::http_client;

#[cfg(test)]
mod tests;
