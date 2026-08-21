use batlehub_core::ports::ExtractedManifest;
use bytes::Bytes;

use super::readme;

/// A `.gem` is a plain tar holding `metadata.gz`, `data.tar.gz` and
/// `checksums.yaml.gz`. The gem's own files — including its README — are inside
/// `data.tar.gz`, so reading one is a double untar.
///
/// There is **no declared README field in a gemspec**: this is a filename
/// convention match, not a protocol guarantee (RFC 0007 open question 3). A gem
/// that names its README something else reports none, which is the honest answer
/// — guessing at `doc/*.rdoc` would show the wrong document.
pub(super) fn extract_rubygems_manifest(data: &Bytes) -> ExtractedManifest {
    ExtractedManifest {
        readme: data_tar(data).and_then(|inner| readme::shallowest_readme(&inner, false)),
        ..ExtractedManifest::default()
    }
}

/// The `data.tar.gz` member of the outer `.gem` tar.
///
/// Read into memory because the inner archive has to be walked, and the outer
/// entry reader is not seekable. Bounded by the same ceiling every other archive
/// read uses — a `.gem` this proxy already buffered whole is bounded by
/// `max_artifact_size_bytes` on the way in, and this bounds what is held twice.
fn data_tar(data: &Bytes) -> Option<Bytes> {
    use batlehub_core::ports::README_EXTRACT_CEILING;
    use std::io::Read;
    use tar::Archive;

    let mut archive = Archive::new(data.as_ref());
    let entries = archive.entries().ok()?;
    for entry in entries.flatten() {
        let Ok(path) = entry.path() else { continue };
        let path = path.to_string_lossy().into_owned();
        if !readme::is_inside_root(&path) || path.trim_start_matches("./") != "data.tar.gz" {
            continue;
        }
        let mut buf = Vec::new();
        // A gem's `data.tar.gz` is the whole package, so this ceiling is a cap
        // on how large a gem can be *and still have its README read*, not on
        // what can be published or served. A gem over 4 MiB reports no README
        // rather than being buffered twice.
        if entry
            .take(README_EXTRACT_CEILING as u64)
            .read_to_end(&mut buf)
            .is_ok()
        {
            return Some(Bytes::from(buf));
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_that_is_not_a_gem_yields_nothing() {
        let data = Bytes::from_static(b"not a gem");
        assert_eq!(
            extract_rubygems_manifest(&data),
            ExtractedManifest::default()
        );
    }

    use super::super::readme::fixtures::{plain_tar, targz};

    /// A `.gem` is a plain tar holding `data.tar.gz`, so the gem's own README is
    /// a double untar away.
    #[test]
    fn the_readme_inside_data_tar_gz_is_found() {
        let inner = targz(&[
            ("lib/mygem.rb", b"module MyGem; end".as_slice()),
            ("README.md", b"# mygem".as_slice()),
        ]);
        let gem = plain_tar(&[
            ("metadata.gz", b"fake".as_slice()),
            ("data.tar.gz", inner.as_ref()),
        ]);
        let readme = extract_rubygems_manifest(&gem).readme.expect("README read");
        assert_eq!(readme.content, "# mygem");
        assert_eq!(readme.path, "README.md");
    }

    /// There is no declared README field in a gemspec, so this is a filename
    /// convention. A gem that names its documentation something else reports
    /// none rather than showing the wrong document.
    #[test]
    fn a_gem_naming_its_docs_something_else_reports_none() {
        let inner = targz(&[("doc/overview.rdoc", b"= Overview".as_slice())]);
        let gem = plain_tar(&[("data.tar.gz", inner.as_ref())]);
        assert!(extract_rubygems_manifest(&gem).readme.is_none());
    }

    /// A `.gem` with no `data.tar.gz` at all — the outer tar is not what we
    /// expected — reports none rather than erroring.
    #[test]
    fn a_gem_without_a_data_member_reports_none() {
        let gem = plain_tar(&[("metadata.gz", b"fake".as_slice())]);
        assert!(extract_rubygems_manifest(&gem).readme.is_none());
    }
}
