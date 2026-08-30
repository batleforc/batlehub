//! A GPG public key a Terraform namespace signs its providers with.

use serde::{Deserialize, Serialize};

/// One entry of the provider download response's `signing_keys.gpg_public_keys`.
///
/// The field names are Terraform's, not this server's: the value is serialised
/// straight into the protocol response, so a rename here is a protocol break.
/// `key_id` and `ascii_armor` are the two the client needs to verify a `SHASUMS`
/// signature; the other three are optional provenance Terraform surfaces in
/// `terraform providers lock` output and are carried through rather than
/// invented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigningKey {
    /// The long key id, as Terraform expects it — uppercase hex, no `0x`.
    pub key_id: String,
    /// The armoured public key block.
    pub ascii_armor: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trust_signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl SigningKey {
    /// Reject a key that cannot verify anything.
    ///
    /// Not a cryptographic check — this server does not parse the key, and
    /// pretending to would be worse than not. It refuses the two shapes that are
    /// definitely useless: an empty id, and a body that is not an armoured
    /// block. A registry that served either would tell Terraform to verify
    /// against nothing while looking like it had been configured, which is the
    /// state the empty placeholder was already in.
    pub fn validate(&self) -> Result<(), String> {
        if self.key_id.trim().is_empty() {
            return Err("key_id must not be empty".to_owned());
        }
        if !self
            .ascii_armor
            .trim_start()
            .starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----")
        {
            return Err(
                "ascii_armor must be an armoured PGP public key block, beginning \
                 '-----BEGIN PGP PUBLIC KEY BLOCK-----'"
                    .to_owned(),
            );
        }
        Ok(())
    }
}
