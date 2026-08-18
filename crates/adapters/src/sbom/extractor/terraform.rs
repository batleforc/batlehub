use batlehub_core::ports::ExtractedManifest;
use bytes::Bytes;

use super::readme;

/// Terraform **modules** are tarballs whose root is the module, and a module's
/// README is what the registry's own page renders. **Providers** are zipped
/// binaries with no README at all, which this answers for by finding no file —
/// no special case is needed, and one that guessed would be worse.
pub(super) fn extract_terraform_manifest(data: &Bytes) -> ExtractedManifest {
    ExtractedManifest {
        readme: readme::shallowest_readme(data, false),
        ..ExtractedManifest::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::readme::fixtures::{targz, zipped};
    use super::*;

    #[test]
    fn a_module_tarballs_readme_is_read() {
        let data = targz(&[
            ("main.tf", b"resource \"null_resource\" \"x\" {}".as_slice()),
            ("README.md", b"# the module".as_slice()),
        ]);
        assert_eq!(
            extract_terraform_manifest(&data).readme.unwrap().content,
            "# the module"
        );
    }

    /// A provider is a zipped binary with no README. It reports none by finding
    /// no file, which is the honest answer — a special case that guessed would
    /// be worse.
    #[test]
    fn a_provider_zip_has_no_readme_to_find() {
        let data = zipped(&[(
            "terraform-provider-null_v3.2.1",
            b"\x7fELF binary".as_slice(),
        )]);
        assert!(extract_terraform_manifest(&data).readme.is_none());
    }
}
