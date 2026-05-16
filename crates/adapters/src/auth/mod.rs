#[cfg(feature = "auth-token")]
pub mod token;
#[cfg(feature = "auth-token")]
pub use token::StaticTokenAuthProvider;

#[cfg(feature = "auth-oidc")]
pub mod oidc;
#[cfg(feature = "auth-oidc")]
pub use oidc::{OidcAuthProvider, OidcSsoFlow, OidcTokens};

#[cfg(feature = "auth-kubernetes")]
pub mod kubernetes;
#[cfg(feature = "auth-kubernetes")]
pub use kubernetes::KubernetesAuthProvider;
