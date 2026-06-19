//! Ed25519 detached-signature verification for downloaded artifacts.
//!
//! The signing framework stores a client-supplied detached signature
//! (`X-Artifact-Signature` + `X-Signature-Type`) per published artifact. When a
//! registry sets `signing.verify_on_download`, a stored `ed25519` signature is
//! re-checked on every download against the registry's configured
//! `trusted_keys` — verifying over the **raw artifact bytes** (the defined
//! scheme for this verifier).
//!
//! Ed25519 is the only signature algorithm verified here on purpose: RSA-based
//! crypto (the `rsa` crate, and therefore PGP/x509/Sigstore default paths) is
//! hard-banned by `deny.toml` (RUSTSEC-2023-0071). Sigstore / npm provenance
//! verification is left as a future item for that reason.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// The signature type label that [`verify_ed25519`] handles.
pub const ED25519_SIG_TYPE: &str = "ed25519";

/// Parse a hex-encoded 32-byte Ed25519 public key.
fn parse_pubkey(hex_key: &str) -> Option<VerifyingKey> {
    let bytes = hex::decode(hex_key.trim()).ok()?;
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}

/// Verify a detached Ed25519 `signature` over `data` against any of the
/// hex-encoded `trusted_keys`.
///
/// Returns `true` only when the signature is a well-formed 64-byte Ed25519
/// signature that verifies under at least one trusted key. Malformed keys are
/// skipped; an empty key list always fails.
pub fn verify_ed25519(trusted_keys: &[String], signature: &[u8], data: &[u8]) -> bool {
    let sig: [u8; 64] = match signature.try_into() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let sig = Signature::from_bytes(&sig);
    trusted_keys
        .iter()
        .filter_map(|k| parse_pubkey(k))
        .any(|vk| vk.verify(data, &sig).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair(seed: u8) -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let pub_hex = hex::encode(sk.verifying_key().to_bytes());
        (sk, pub_hex)
    }

    #[test]
    fn valid_signature_with_trusted_key_verifies() {
        let (sk, pub_hex) = keypair(7);
        let data = b"artifact bytes";
        let sig = sk.sign(data).to_bytes().to_vec();
        assert!(verify_ed25519(&[pub_hex], &sig, data));
    }

    #[test]
    fn untrusted_key_fails() {
        let (sk, _) = keypair(7);
        let (_, other_pub) = keypair(9);
        let data = b"artifact bytes";
        let sig = sk.sign(data).to_bytes().to_vec();
        assert!(!verify_ed25519(&[other_pub], &sig, data));
    }

    #[test]
    fn tampered_data_fails() {
        let (sk, pub_hex) = keypair(7);
        let sig = sk.sign(b"original").to_bytes().to_vec();
        assert!(!verify_ed25519(&[pub_hex], &sig, b"tampered"));
    }

    #[test]
    fn one_trusted_key_among_many_passes() {
        let (sk, pub_hex) = keypair(3);
        let (_, other) = keypair(4);
        let data = b"abc";
        let sig = sk.sign(data).to_bytes().to_vec();
        assert!(verify_ed25519(&[other, pub_hex], &sig, data));
    }

    #[test]
    fn malformed_signature_length_fails() {
        let (_, pub_hex) = keypair(7);
        assert!(!verify_ed25519(&[pub_hex], &[1, 2, 3], b"data"));
    }

    #[test]
    fn empty_trusted_keys_fails() {
        let (sk, _) = keypair(7);
        let data = b"abc";
        let sig = sk.sign(data).to_bytes().to_vec();
        assert!(!verify_ed25519(&[], &sig, data));
    }

    #[test]
    fn malformed_trusted_key_is_skipped() {
        let (sk, pub_hex) = keypair(7);
        let data = b"abc";
        let sig = sk.sign(data).to_bytes().to_vec();
        // A bogus key entry must not break verification against a good one.
        assert!(verify_ed25519(
            &["not-hex".to_string(), pub_hex],
            &sig,
            data
        ));
    }
}
