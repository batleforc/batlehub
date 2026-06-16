//! Debian/RPM repository hosting primitives: package parsing, index generation,
//! and Ed25519 OpenPGP signing.

pub mod openpgp;

#[cfg(feature = "registry-deb")]
pub mod deb;

#[cfg(feature = "registry-rpm")]
pub mod rpm;

#[cfg(feature = "registry-pacman")]
pub mod pacman;

pub use openpgp::OpenPgpSigner;

/// Gzip a byte slice (used for `Packages.gz` and the `repodata/*.xml.gz` files).
pub fn gzip(data: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)?;
    enc.finish()
}
