use bytes::Bytes;

use batlehub_core::ports::ExtractedManifest;

mod cargo;
mod maven;
mod npm;
mod nuget;
mod pypi;

/// Archive-based SBOM manifest extractor.
///
/// Parses dependency manifests embedded in package archives, and the licence
/// the same manifest declares (RFC 0004-bis §13.1).
/// Requires the `sbom` feature (which enables flate2, tar, zip, quick-xml).
pub struct ArchiveSbomExtractor;

impl batlehub_core::ports::SbomExtractor for ArchiveSbomExtractor {
    fn extract(&self, data: &Bytes, registry_type: &str) -> ExtractedManifest {
        match registry_type {
            "cargo" => cargo::extract_cargo_manifest(data),
            "npm" => npm::extract_npm_manifest(data),
            "maven" => maven::extract_maven_manifest(data),
            "pypi" => pypi::extract_pypi_manifest(data),
            "nuget" => nuget::extract_nuget_manifest(data),
            // The other sixteen registry types have no manifest parser, so
            // they report an unknown licence rather than an absent one — which
            // is why `license_gate.allow_unknown` defaults to true.
            _ => ExtractedManifest::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use batlehub_core::ports::SbomExtractor;

    use super::*;

    #[test]
    fn extract_returns_empty_for_unknown_type() {
        let extractor = ArchiveSbomExtractor;
        let data = Bytes::from_static(b"not an archive");
        assert_eq!(
            extractor.extract(&data, "unknown"),
            ExtractedManifest::default()
        );
    }

    /// The dispatch above and `LICENSE_EXTRACTION_TYPES` must not drift.
    ///
    /// The config warning that tells an operator their `license_gate` cannot
    /// see a licence is derived from the const, so a parser added here without
    /// updating it warns about a registry type that now works — and a type
    /// added there without a parser silences a warning that is still true.
    /// Either way the operator is told something false about their policy,
    /// which is the failure mode RFC 0004-bis exists to stop.
    #[test]
    fn dispatch_matches_the_declared_extraction_types() {
        // Mirror of the match arms, in the same order as the const.
        let dispatched = ["cargo", "maven", "npm", "nuget", "pypi"];
        assert_eq!(
            dispatched.as_slice(),
            batlehub_core::ports::LICENSE_EXTRACTION_TYPES,
            "update LICENSE_EXTRACTION_TYPES when adding or removing a parser"
        );
    }
}
