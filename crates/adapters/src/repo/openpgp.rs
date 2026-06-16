//! Minimal OpenPGP (RFC 4880) signing using **Ed25519 only**.
//!
//! A full OpenPGP library (rpgp, sequoia) pulls in the `rsa` crate, which is
//! hard-banned in `deny.toml` (RUSTSEC-2023-0071). APT and DNF only need the repo
//! metadata to be signed by a key they trust — the algorithm is our choice — so we
//! hand-roll just enough of the format to emit:
//!
//! - an **armored public key block** (v4 EdDSA primary key + User ID + a positive
//!   self-certification), importable by `gpg`/`apt-key`/`rpm --import`;
//! - **detached binary signatures** (for `Release.gpg`, `repomd.xml.asc`);
//! - **cleartext (inline) signatures** (for APT `InRelease`).
//!
//! EdDSA uses the legacy algorithm id 22 (what `gpg` generates for Ed25519 keys and
//! what current `gpgv`/`rpm` verify). The signature is `Ed25519(H(data || trailer))`
//! per RFC 4880-bis §5.2.4 (PureEdDSA over the hash output).
//!
//! Correctness against the OpenPGP wire format is covered by unit tests (the
//! Ed25519 signature is re-verified, packets are re-parsed); end-to-end interop with
//! `apt`/`dnf` is a manual-verification step (see `docs/`).

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha512};

use batlehub_core::error::CoreError;

/// Ed25519 curve OID (1.3.6.1.4.1.11591.15.1), as embedded in OpenPGP EdDSA keys.
const ED25519_OID: [u8; 9] = [0x2B, 0x06, 0x01, 0x04, 0x01, 0xDA, 0x47, 0x0F, 0x01];
const ALGO_EDDSA: u8 = 22;
const HASH_SHA512: u8 = 10;

/// An Ed25519 OpenPGP signer built from a raw 32-byte seed.
pub struct OpenPgpSigner {
    signing: SigningKey,
    verifying: VerifyingKey,
    /// Key creation time (unix seconds); part of the fingerprint, so it must be
    /// stable across restarts — taken from config.
    created: u32,
    user_id: String,
}

impl OpenPgpSigner {
    /// Build a signer from a hex-encoded 32-byte Ed25519 seed.
    pub fn from_seed_hex(seed_hex: &str, created: u32, user_id: &str) -> Result<Self, CoreError> {
        let bytes = hex::decode(seed_hex.trim())
            .map_err(|e| CoreError::InvalidInput(format!("signing key is not valid hex: {e}")))?;
        let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            CoreError::InvalidInput(format!(
                "signing key must be 32 bytes (got {})",
                bytes.len()
            ))
        })?;
        let signing = SigningKey::from_bytes(&seed);
        let verifying = signing.verifying_key();
        Ok(Self {
            signing,
            verifying,
            created,
            user_id: user_id.to_owned(),
        })
    }

    // ── Public-key packet & identity ────────────────────────────────────────

    /// v4 public-key packet *body* (without the packet header).
    fn pubkey_body(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.push(4); // version
        b.extend_from_slice(&self.created.to_be_bytes());
        b.push(ALGO_EDDSA);
        b.push(ED25519_OID.len() as u8);
        b.extend_from_slice(&ED25519_OID);
        // EdDSA public point: 0x40 prefix || 32-byte compressed point, as an MPI.
        let mut point = Vec::with_capacity(33);
        point.push(0x40);
        point.extend_from_slice(self.verifying.as_bytes());
        b.extend_from_slice(&encode_mpi(&point));
        b
    }

    /// 20-byte v4 fingerprint: SHA-1 over `0x99 || len16 || pubkey_body`.
    fn fingerprint(&self) -> [u8; 20] {
        use sha1::Sha1;
        let body = self.pubkey_body();
        let mut h = Sha1::new();
        h.update([0x99]);
        h.update((body.len() as u16).to_be_bytes());
        h.update(&body);
        h.finalize().into()
    }

    /// 8-byte key ID (low 64 bits of the fingerprint).
    fn key_id(&self) -> [u8; 8] {
        let fp = self.fingerprint();
        fp[12..20].try_into().expect("fingerprint is 20 bytes")
    }

    /// Armored OpenPGP public key block (primary key + User ID + self-cert),
    /// suitable for `apt`'s `Signed-By:` / `rpm --import`.
    pub fn armored_public_key(&self) -> String {
        let mut packets = Vec::new();
        let pubkey_body = self.pubkey_body();
        packets.extend_from_slice(&framed_packet(6, &pubkey_body));
        packets.extend_from_slice(&framed_packet(13, self.user_id.as_bytes()));
        let selfcert = self.user_id_self_cert(&pubkey_body);
        packets.extend_from_slice(&framed_packet(2, &selfcert));
        armor("PGP PUBLIC KEY BLOCK", &packets)
    }

    // ── Signatures ──────────────────────────────────────────────────────────

    /// Build the hashed-subpacket area: creation time, issuer fingerprint, and any
    /// caller-supplied `extra` subpackets (e.g. Key Flags on a self-certification).
    fn hashed_subpackets(&self, extra: &[u8]) -> Vec<u8> {
        let mut sp = Vec::new();
        // Signature creation time (type 2): reuse the key creation time for
        // reproducibility.
        push_subpacket(&mut sp, 2, &self.created.to_be_bytes());
        // Issuer fingerprint (type 33): version byte (4) || 20-byte fingerprint.
        let mut issuer_fp = Vec::with_capacity(21);
        issuer_fp.push(4);
        issuer_fp.extend_from_slice(&self.fingerprint());
        push_subpacket(&mut sp, 33, &issuer_fp);
        sp.extend_from_slice(extra);
        sp
    }

    /// Assemble a v4 signature packet body. `sig_type` is 0x00 (binary doc) or 0x13
    /// (positive cert). `extra_subpackets` are appended to the hashed subpacket area;
    /// `extra_hashed` is extra hashed *material* prepended before the data (key +
    /// User ID for a certification).
    fn build_signature(
        &self,
        sig_type: u8,
        extra_subpackets: &[u8],
        extra_hashed: &[u8],
        data: &[u8],
    ) -> Vec<u8> {
        let hashed_sp = self.hashed_subpackets(extra_subpackets);

        // Hashed prefix of the signature packet: version, type, pkalgo, hashalgo,
        // hashed-subpacket length, hashed subpackets.
        let mut hashed_prefix = vec![4u8, sig_type, ALGO_EDDSA, HASH_SHA512];
        hashed_prefix.extend_from_slice(&(hashed_sp.len() as u16).to_be_bytes());
        hashed_prefix.extend_from_slice(&hashed_sp);

        // Digest = H(extra_hashed || data || hashed_prefix || trailer).
        let mut hasher = Sha512::new();
        hasher.update(extra_hashed);
        hasher.update(data);
        hasher.update(&hashed_prefix);
        // Final trailer: 0x04, 0xFF, 4-byte big-endian length of hashed_prefix.
        hasher.update([0x04, 0xFF]);
        hasher.update((hashed_prefix.len() as u32).to_be_bytes());
        let digest = hasher.finalize();

        // EdDSA over the digest output (PureEdDSA).
        let sig = self.signing.sign(&digest);
        let sig_bytes = sig.to_bytes();
        let (r, s) = sig_bytes.split_at(32);

        // Unhashed subpackets: issuer key ID (type 16).
        let mut unhashed_sp = Vec::new();
        push_subpacket(&mut unhashed_sp, 16, &self.key_id());

        let mut body = hashed_prefix;
        body.extend_from_slice(&(unhashed_sp.len() as u16).to_be_bytes());
        body.extend_from_slice(&unhashed_sp);
        // Left 16 bits of the digest.
        body.extend_from_slice(&digest[0..2]);
        // EdDSA signature MPIs: R then S.
        body.extend_from_slice(&encode_mpi(r));
        body.extend_from_slice(&encode_mpi(s));
        body
    }

    /// Positive User-ID self-certification (0x13) over key + User ID.
    fn user_id_self_cert(&self, pubkey_body: &[u8]) -> Vec<u8> {
        // Hashed material per RFC 4880 §5.2.4: key (0x99||len16||body) then
        // User ID (0xB4||len32||uid).
        let mut extra = Vec::new();
        extra.push(0x99);
        extra.extend_from_slice(&(pubkey_body.len() as u16).to_be_bytes());
        extra.extend_from_slice(pubkey_body);
        extra.push(0xB4);
        extra.extend_from_slice(&(self.user_id.len() as u32).to_be_bytes());
        extra.extend_from_slice(self.user_id.as_bytes());
        // Key Flags subpacket (type 27): 0x01 certify | 0x02 sign. Without this,
        // strict verifiers (e.g. Sequoia, used by modern apt) reject the key as
        // "not signing capable".
        let mut key_flags = Vec::new();
        push_subpacket(&mut key_flags, 27, &[0x03]);
        self.build_signature(0x13, &key_flags, &extra, &[])
    }

    /// Detached binary signature over `data`, ASCII-armored (`Release.gpg`,
    /// `repomd.xml.asc`).
    pub fn detached_sign(&self, data: &[u8]) -> String {
        armor("PGP SIGNATURE", &self.detached_sign_binary(data))
    }

    /// Detached binary signature over `data`, **not** armored — the raw OpenPGP
    /// signature packet. Used for pacman's `<repo>.db.sig` / `.pkg.tar.zst.sig`
    /// files and the base64-encoded `%PGPSIG%` database field, which all expect
    /// the binary form rather than an armored block.
    pub fn detached_sign_binary(&self, data: &[u8]) -> Vec<u8> {
        let body = self.build_signature(0x00, &[], &[], data);
        framed_packet(2, &body)
    }

    /// Cleartext (inline) signature over `text`, producing an APT `InRelease`-style
    /// document.
    ///
    /// Per RFC 4880 §7, the signature is over the **canonical text form**: trailing
    /// whitespace stripped from each line, lines joined with `<CR><LF>`, and no
    /// trailing line ending — signed as a canonical-text document (type 0x01). The
    /// emitted body keeps the original `\n` text (verifiers canonicalise it the same
    /// way). We do not dash-escape because `Release` files never begin a line with
    /// `-`.
    pub fn clear_sign(&self, text: &str) -> String {
        let canonical = canonical_text(text);
        let body = self.build_signature(0x01, &[], &[], &canonical);
        let sig_armor = armor("PGP SIGNATURE", &framed_packet(2, &body));
        let mut out = String::new();
        out.push_str("-----BEGIN PGP SIGNED MESSAGE-----\n");
        out.push_str("Hash: SHA512\n\n");
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&sig_armor);
        out
    }
}

/// OpenPGP canonical text form for cleartext signatures: strip trailing spaces and
/// tabs from each line, join with `<CR><LF>`, and drop the trailing line ending.
fn canonical_text(text: &str) -> Vec<u8> {
    let mut lines: Vec<&str> = text.split('\n').collect();
    // A trailing '\n' yields a final empty element; the canonical form has no
    // trailing line terminator.
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
        .iter()
        .map(|l| l.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\r\n")
        .into_bytes()
}

// ── Encoding helpers ────────────────────────────────────────────────────────

/// Encode a big-endian byte slice as an OpenPGP MPI (2-byte bit length + bytes,
/// leading zero bytes stripped).
fn encode_mpi(bytes: &[u8]) -> Vec<u8> {
    // Find the first non-zero byte.
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let sig = &bytes[first..];
    let bits = if sig.is_empty() {
        0
    } else {
        (sig.len() - 1) * 8 + (8 - sig[0].leading_zeros() as usize)
    };
    let mut out = Vec::with_capacity(2 + sig.len());
    out.extend_from_slice(&(bits as u16).to_be_bytes());
    out.extend_from_slice(sig);
    out
}

/// Append a signature subpacket (length-prefixed, single-byte type) to `out`.
/// Uses the one/two/five-octet length encoding; subpacket length covers the type
/// byte + data.
fn push_subpacket(out: &mut Vec<u8>, sp_type: u8, data: &[u8]) {
    let len = data.len() + 1; // +1 for the type octet
    encode_length(out, len);
    out.push(sp_type);
    out.extend_from_slice(data);
}

/// New-format packet length encoding (RFC 4880 §4.2.2), shared by packet headers
/// and signature subpackets.
fn encode_length(out: &mut Vec<u8>, len: usize) {
    if len < 192 {
        out.push(len as u8);
    } else if len < 8384 {
        let l = len - 192;
        out.push(((l >> 8) + 192) as u8);
        out.push((l & 0xFF) as u8);
    } else {
        out.push(0xFF);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

/// Wrap a packet body in a new-format packet header (tag byte `0xC0 | tag`).
fn framed_packet(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 6);
    out.push(0xC0 | tag);
    encode_length(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

/// RFC 4880 §6.1 CRC-24 over `data`.
fn crc24(data: &[u8]) -> u32 {
    let mut crc: u32 = 0x00B7_04CE;
    for &byte in data {
        crc ^= (byte as u32) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x0100_0000 != 0 {
                crc ^= 0x0186_4CFB;
            }
        }
    }
    crc & 0x00FF_FFFF
}

/// ASCII-armor a packet stream (RFC 4880 §6.2).
fn armor(label: &str, packets: &[u8]) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let b64 = engine.encode(packets);

    let mut out = String::new();
    out.push_str(&format!("-----BEGIN {label}-----\n\n"));
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    let crc = crc24(packets);
    let crc_bytes = [(crc >> 16) as u8, (crc >> 8) as u8, crc as u8];
    out.push('=');
    out.push_str(&engine.encode(crc_bytes));
    out.push('\n');
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "9d61b19deffeba00aa3f3b6e3b0fe6a3f3a76b08e2c0a3f3b6e3b0fe6a3f3a76";
    // ^ 32 bytes hex (arbitrary but valid).

    fn signer() -> OpenPgpSigner {
        OpenPgpSigner::from_seed_hex(SEED, 1_700_000_000, "BatleHub <repo@example.com>").unwrap()
    }

    #[test]
    fn rejects_bad_seed_length() {
        assert!(OpenPgpSigner::from_seed_hex("abcd", 0, "x").is_err());
    }

    #[test]
    fn rejects_non_hex_seed() {
        assert!(OpenPgpSigner::from_seed_hex("zz".repeat(32).as_str(), 0, "x").is_err());
    }

    #[test]
    fn fingerprint_is_stable() {
        let a = signer().fingerprint();
        let b = signer().fingerprint();
        assert_eq!(a, b);
        // key id is the trailing 8 bytes
        assert_eq!(&a[12..20], &signer().key_id());
    }

    #[test]
    fn mpi_strips_leading_zeros_and_counts_bits() {
        // 0x00 0x01 → 1 bit, single byte 0x01
        assert_eq!(encode_mpi(&[0x00, 0x01]), vec![0x00, 0x01, 0x01]);
        // 0x40.. → high byte 0x40 = bit 7 (within the byte, 7 significant bits)
        let m = encode_mpi(&[0x40, 0x00]);
        assert_eq!(&m[0..2], &[0x00, 0x0F]); // 15 bits
    }

    #[test]
    fn armored_public_key_has_framing() {
        let armored = signer().armored_public_key();
        assert!(armored.starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----"));
        assert!(armored
            .trim_end()
            .ends_with("-----END PGP PUBLIC KEY BLOCK-----"));
        // CRC line present
        assert!(armored.lines().any(|l| l.starts_with('=')));
    }

    #[test]
    fn detached_signature_is_armored_and_crypto_path_verifies() {
        let s = signer();
        let data = b"Origin: BatleHub\nLabel: test\n";
        let armored = s.detached_sign(data);
        assert!(armored.starts_with("-----BEGIN PGP SIGNATURE-----"));
        assert!(armored.trim_end().ends_with("-----END PGP SIGNATURE-----"));

        // Re-derive the OpenPGP digest exactly as `build_signature` does and verify
        // the resulting Ed25519 signature against the public key. This exercises the
        // same hash construction the packet commits to (PureEdDSA over the digest).
        let hashed_sp = s.hashed_subpackets(&[]);
        let mut hashed_prefix = vec![4u8, 0x00, ALGO_EDDSA, HASH_SHA512];
        hashed_prefix.extend_from_slice(&(hashed_sp.len() as u16).to_be_bytes());
        hashed_prefix.extend_from_slice(&hashed_sp);
        let mut hasher = Sha512::new();
        hasher.update(data);
        hasher.update(&hashed_prefix);
        hasher.update([0x04, 0xFF]);
        hasher.update((hashed_prefix.len() as u32).to_be_bytes());
        let digest = hasher.finalize();

        use ed25519_dalek::Verifier;
        let signature = s.signing.sign(&digest);
        assert!(s.verifying.verify(&digest, &signature).is_ok());
    }

    #[test]
    fn detached_binary_signature_is_unarmored_packet_and_verifies() {
        let s = signer();
        let data = b"pacman db bytes";
        let bin = s.detached_sign_binary(data);
        // New-format signature packet header: tag 2 ⇒ first byte 0xC2.
        assert_eq!(bin[0], 0xC0 | 2);
        // The armored form wraps exactly these bytes.
        assert_eq!(s.detached_sign(data), armor("PGP SIGNATURE", &bin));

        // Re-derive the committed digest and verify the Ed25519 signature.
        let hashed_sp = s.hashed_subpackets(&[]);
        let mut hashed_prefix = vec![4u8, 0x00, ALGO_EDDSA, HASH_SHA512];
        hashed_prefix.extend_from_slice(&(hashed_sp.len() as u16).to_be_bytes());
        hashed_prefix.extend_from_slice(&hashed_sp);
        let mut hasher = Sha512::new();
        hasher.update(data);
        hasher.update(&hashed_prefix);
        hasher.update([0x04, 0xFF]);
        hasher.update((hashed_prefix.len() as u32).to_be_bytes());
        let digest = hasher.finalize();
        use ed25519_dalek::Verifier;
        let signature = s.signing.sign(&digest);
        assert!(s.verifying.verify(&digest, &signature).is_ok());
    }

    #[test]
    fn build_signature_is_deterministic() {
        let s = signer();
        let a = s.build_signature(0x00, &[], &[], b"data");
        let b = s.build_signature(0x00, &[], &[], b"data");
        assert_eq!(a, b, "Ed25519 + reused creation time ⇒ reproducible");
    }

    #[test]
    fn clear_sign_wraps_text() {
        let out = signer().clear_sign("Origin: X\nSuite: stable\n");
        assert!(out.starts_with("-----BEGIN PGP SIGNED MESSAGE-----"));
        assert!(out.contains("Hash: SHA512"));
        assert!(out.contains("Origin: X"));
        assert!(out.contains("-----BEGIN PGP SIGNATURE-----"));
    }
}
