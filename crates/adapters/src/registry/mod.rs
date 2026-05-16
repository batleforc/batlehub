pub mod fanout;
pub use fanout::FanoutRegistryClient;

#[cfg(feature = "registry-github")]
pub mod github;
#[cfg(feature = "registry-github")]
pub use github::GithubRegistryClient;

#[cfg(feature = "registry-npm")]
pub mod npm;
#[cfg(feature = "registry-npm")]
pub use npm::NpmRegistryClient;

#[cfg(feature = "registry-cargo")]
pub mod cargo;
#[cfg(feature = "registry-cargo")]
pub use cargo::CargoRegistryClient;
