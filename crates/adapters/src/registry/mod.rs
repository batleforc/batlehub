pub mod http_client;
pub use http_client::{
    apply_upstream_options, apply_upstream_tls, percent_encode, upstream_auth_headers,
    UpstreamHttpOptions,
};

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

#[cfg(feature = "registry-nuget")]
pub mod nuget;
#[cfg(feature = "registry-nuget")]
pub use nuget::NugetRegistryClient;

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

#[cfg(feature = "registry-terraform")]
pub mod terraform;
#[cfg(feature = "registry-terraform")]
pub use terraform::TerraformRegistryClient;

#[cfg(feature = "registry-rubygems")]
pub mod rubygems;
#[cfg(feature = "registry-rubygems")]
pub use rubygems::RubyGemsRegistryClient;

#[cfg(feature = "registry-composer")]
pub mod composer;
#[cfg(feature = "registry-composer")]
pub use composer::ComposerRegistryClient;

#[cfg(feature = "registry-pypi")]
pub mod pypi;
#[cfg(feature = "registry-pypi")]
pub use pypi::PypiRegistryClient;

#[cfg(feature = "registry-conda")]
pub mod conda;
#[cfg(feature = "registry-conda")]
pub use conda::CondaRegistryClient;
