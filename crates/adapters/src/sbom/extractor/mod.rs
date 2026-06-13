use bytes::Bytes;

use batlehub_core::ports::SbomDependency;

mod cargo;
mod maven;
mod npm;
mod nuget;
mod pypi;

/// Archive-based SBOM dependency extractor.
///
/// Parses dependency manifests embedded in package archives.
/// Requires the `sbom` feature (which enables flate2, tar, zip, quick-xml).
pub struct ArchiveSbomExtractor;

impl batlehub_core::ports::SbomExtractor for ArchiveSbomExtractor {
    fn extract(&self, data: &Bytes, registry_type: &str) -> Vec<SbomDependency> {
        match registry_type {
            "cargo" => cargo::extract_cargo_deps(data),
            "npm" => npm::extract_npm_deps(data),
            "maven" => maven::extract_maven_deps(data),
            "pypi" => pypi::extract_pypi_deps(data),
            "nuget" => nuget::extract_nuget_deps(data),
            _ => vec![],
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
        assert!(extractor.extract(&data, "unknown").is_empty());
    }
}
