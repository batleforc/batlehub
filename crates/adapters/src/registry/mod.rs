pub mod http_client;
pub use http_client::{UpstreamHttpOptions, apply_upstream_options, apply_upstream_tls, upstream_auth_headers};

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

#[cfg(feature = "registry-openvsx")]
pub mod openvsx;
#[cfg(feature = "registry-openvsx")]
pub use openvsx::OpenVsxRegistryClient;

#[cfg(feature = "registry-goproxy")]
pub mod goproxy;
#[cfg(feature = "registry-goproxy")]
pub use goproxy::GoProxyRegistryClient;

#[cfg(feature = "registry-vscode-marketplace")]
pub mod vscode_marketplace;
#[cfg(feature = "registry-vscode-marketplace")]
pub use vscode_marketplace::VsCodeMarketplaceRegistryClient;

#[cfg(feature = "registry-maven")]
pub mod maven;
#[cfg(feature = "registry-maven")]
pub use maven::MavenRegistryClient;
