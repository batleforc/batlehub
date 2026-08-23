use batlehub_core::ports::ExtractedManifest;
use bytes::Bytes;

use super::readme;

/// A Composer dist zip is usually a GitHub source archive, so its single
/// wrapper directory is named `{vendor}-{repo}-{sha}` — but a zip built by
/// `composer archive` has no wrapper at all. The README is the shallowest
/// `README*` rather than one at a fixed depth.
pub(super) fn extract_composer_manifest(data: &Bytes) -> ExtractedManifest {
    ExtractedManifest {
        readme: readme::shallowest_readme(data, true),
        ..ExtractedManifest::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::readme::fixtures::zipped;
    use super::*;

    /// A GitHub source archive: one wrapper directory named after the commit.
    #[test]
    fn a_github_style_dist_zip_is_read_through_its_wrapper() {
        let data = zipped(&[
            ("vendor-pkg-abc1234/composer.json", b"{}".as_slice()),
            ("vendor-pkg-abc1234/README.md", b"# the package".as_slice()),
        ]);
        assert_eq!(
            extract_composer_manifest(&data).readme.unwrap().content,
            "# the package"
        );
    }

    /// A zip built by `composer archive` has no wrapper at all.
    #[test]
    fn an_unwrapped_dist_zip_is_read_at_the_root() {
        let data = zipped(&[("README.md", b"# unwrapped".as_slice())]);
        assert_eq!(
            extract_composer_manifest(&data).readme.unwrap().content,
            "# unwrapped"
        );
    }
}
