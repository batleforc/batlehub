use batlehub_core::ports::ExtractedManifest;
use bytes::Bytes;

use super::readme;

/// A Go module `.zip` carries no manifest this extractor reads a licence or
/// dependencies out of — `go.mod` is a dependency list, but the SBOM side has
/// never parsed it and this RFC does not add that — so the README is the only
/// fact returned here.
///
/// Every path inside a module zip is prefixed `{module}@{version}/`, and the
/// module path itself contains slashes (`github.com/user/repo@v1.0.0/README.md`),
/// so the wrapper depth is not fixed. The README is the shallowest `README*`.
pub(super) fn extract_goproxy_manifest(data: &Bytes) -> ExtractedManifest {
    ExtractedManifest {
        readme: readme::shallowest_readme(data, true),
        ..ExtractedManifest::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::readme::fixtures::zipped;
    use super::*;

    #[test]
    fn the_module_roots_readme_is_read_through_the_versioned_prefix() {
        let data = zipped(&[
            (
                "github.com/user/repo@v1.2.3/go.mod",
                b"module github.com/user/repo".as_slice(),
            ),
            (
                "github.com/user/repo@v1.2.3/README.md",
                b"# the module".as_slice(),
            ),
            (
                "github.com/user/repo@v1.2.3/internal/README.md",
                b"# internal".as_slice(),
            ),
        ]);
        let readme = extract_goproxy_manifest(&data).readme.expect("README read");
        assert_eq!(readme.content, "# the module");
    }

    /// Go module zips carry no manifest this reads a licence out of, and saying
    /// so is the point: `LICENSE_EXTRACTION_TYPES` does not list `goproxy`, so
    /// a `license_gate` on a Go registry is warned about rather than silently
    /// inert.
    #[test]
    fn a_module_answers_for_its_readme_and_nothing_else() {
        let data = zipped(&[("m@v1/README.md", b"# m".as_slice())]);
        let manifest = extract_goproxy_manifest(&data);
        assert!(manifest.readme.is_some());
        assert!(manifest.license.is_none());
        assert!(manifest.dependencies.is_empty());
    }
}
