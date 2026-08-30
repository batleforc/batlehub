//! The GPG keys a Terraform namespace's providers are signed with.
//!
//! RFC 0015 §4.2 names `terraform:signing-keys:write` as the verb with no
//! equivalent elsewhere — *"registering the GPG key a namespace's providers are
//! signed with"* — and §13.13 recorded it as one of four verbs gating an action
//! this server did not implement.
//!
//! # This was a hole, not just a missing verb
//!
//! Terraform verifies a provider's `SHASUMS` signature against the keys the
//! registry serves in its download response, and
//! `eco_terraform.rs` served `{"gpg_public_keys": []}` — a hardcoded placeholder.
//! An empty list is not "unsigned, proceed": it is a registry telling the client
//! there is nothing to verify against, so **no locally published provider could
//! be verified by anybody**. The verb was absent because the store was, and the
//! store being absent was a supply-chain gap rather than a missing feature.
//!
//! # Keyed by namespace, which is the tier Terraform signs at
//!
//! A publisher signs every provider under their namespace with one key, which is
//! why §4.2 words the action as *"a namespace's providers"*. That happens to be
//! RFC 0015 §4.1's namespace tier as well, so the grant an operator writes to
//! delegate this — `terraform:signing-keys:write` on `[[registries.namespaces]]`
//! — names exactly the scope the key covers.

use async_trait::async_trait;

use crate::entities::SigningKey;
use crate::error::CoreError;

/// Storage for a namespace's provider signing keys.
#[async_trait]
pub trait SigningKeyPort: Send + Sync {
    /// Every key registered for `namespace`, in insertion order.
    ///
    /// A namespace with none returns an empty vector, which the download
    /// response serves as it always did. That is the pre-existing behaviour and
    /// it is deliberately kept: refusing to serve a provider whose namespace has
    /// registered no key would break every local Terraform registry on upgrade,
    /// which §10 forbids. See `set_signing_key`'s note for the setting that
    /// would make it refusable.
    async fn list_signing_keys(
        &self,
        registry: &str,
        namespace: &str,
    ) -> Result<Vec<SigningKey>, CoreError>;

    /// Register or replace a key by its id.
    ///
    /// # Why publishing without one is still allowed
    ///
    /// The coherent end state is that a namespace declaring a key refuses a
    /// provider it cannot verify — but making that the *default* is a behaviour
    /// change that breaks every estate publishing Terraform providers today, and
    /// §10's promise is that no existing config changes meaning. So this ships
    /// as "register keys and they are served"; the refusal is a
    /// `require_signing_keys` setting on the namespace's `versioning` block when
    /// somebody wants it, which is a new field rather than a new model.
    async fn set_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key: SigningKey,
    ) -> Result<(), CoreError>;

    /// Remove a key by id. Absent is not an error.
    async fn delete_signing_key(
        &self,
        registry: &str,
        namespace: &str,
        key_id: &str,
    ) -> Result<(), CoreError>;
}
