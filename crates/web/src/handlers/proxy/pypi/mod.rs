pub mod publish;
pub mod simple;

pub use publish::pypi_publish;
pub use simple::{pypi_file_download, pypi_json, pypi_simple_package, pypi_simple_root};

/// Parse a PyPI distribution filename into `(normalized_name, version)`.
///
/// Handles wheel (`name-version-py-abi-platform.whl`) and sdist
/// (`name-version.tar.gz`, `name-version.zip`) formats.  Returns `None` if
/// the filename cannot be parsed.
pub fn parse_pypi_filename(filename: &str) -> Option<(String, String)> {
    // Strip known extensions to get the stem
    let stem = filename
        .strip_suffix(".whl")
        .or_else(|| filename.strip_suffix(".tar.gz"))
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .or_else(|| filename.strip_suffix(".zip"))?;

    // Split on '-' and find the first segment that starts with a digit — that's the version
    let parts: Vec<&str> = stem.split('-').collect();
    for i in 1..parts.len() {
        if parts[i].starts_with(|c: char| c.is_ascii_digit()) {
            let name = batlehub_adapters::registry::pypi::normalize_name(&parts[..i].join("-"));
            let version = parts[i].to_owned();
            return Some((name, version));
        }
    }
    None
}

/// Longest distribution filename accepted on publish. PyPI's own longest names
/// are well under this; the bound exists so a pathological value cannot be
/// stored and re-served forever.
const MAX_FILENAME_LEN: usize = 256;

/// Reject an uploaded distribution filename that is not a plain PEP 427/625
/// name, **before** it is persisted into `index_metadata`.
///
/// The Simple index escapes what it interpolates, so this is not what stops the
/// XSS — it is what stops a hostile value from being stored at all, so nothing
/// downstream (a future template, an export, a client that renders the raw
/// metadata) inherits the problem. Two rules: the character set real
/// distribution filenames use, and `parse_pypi_filename` agreeing that the
/// result names a distribution.
pub fn validate_distribution_filename(filename: &str) -> Result<(), String> {
    if filename.is_empty() {
        return Err("distribution filename must not be empty".to_owned());
    }
    if filename.len() > MAX_FILENAME_LEN {
        return Err(format!(
            "distribution filename exceeds {MAX_FILENAME_LEN} characters"
        ));
    }
    if let Some(bad) = filename
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+')))
    {
        return Err(format!(
            "distribution filename contains an illegal character '{bad}'"
        ));
    }
    if filename.starts_with('.') || filename.contains("..") {
        return Err("distribution filename must not contain a path-traversal segment".to_owned());
    }
    if parse_pypi_filename(filename).is_none() {
        return Err(format!(
            "cannot parse PyPI distribution filename: {filename}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wheel_filename() {
        let (name, version) = parse_pypi_filename("requests-2.28.0-py3-none-any.whl").unwrap();
        assert_eq!(name, "requests");
        assert_eq!(version, "2.28.0");
    }

    #[test]
    fn parse_sdist_tar_gz() {
        let (name, version) = parse_pypi_filename("requests-2.28.0.tar.gz").unwrap();
        assert_eq!(name, "requests");
        assert_eq!(version, "2.28.0");
    }

    #[test]
    fn parse_hyphenated_package_name() {
        let (name, version) = parse_pypi_filename("my-cool-package-1.0.0.tar.gz").unwrap();
        assert_eq!(name, "my-cool-package");
        assert_eq!(version, "1.0.0");
    }

    #[test]
    fn parse_invalid_filename_returns_none() {
        assert!(parse_pypi_filename("notapackage.exe").is_none());
    }

    #[test]
    fn validate_accepts_real_distribution_filenames() {
        for name in [
            "requests-2.28.0.tar.gz",
            "requests-2.28.0-py3-none-any.whl",
            "my_cool_package-1.0.0.zip",
            "torch-2.1.0+cu118-cp311-cp311-linux_x86_64.whl",
            "pkg-1.0.0.tar.bz2",
        ] {
            assert!(
                validate_distribution_filename(name).is_ok(),
                "rejected a legitimate filename: {name}"
            );
        }
    }

    /// The markup half of the stored-XSS primitive: this filename *parses*
    /// (`<img …>` then `1.0.0`), so the character-set rule is what rejects it.
    #[test]
    fn validate_rejects_markup_in_a_parseable_filename() {
        let hostile = "<img src=x onerror=alert(1)>-1.0.0.tar.gz";
        assert!(parse_pypi_filename(hostile).is_some());
        assert!(validate_distribution_filename(hostile).is_err());
    }

    #[test]
    fn validate_rejects_the_anchor_injection_payload() {
        assert!(validate_distribution_filename(
            "evil-1.0.tar.gz</a><a href=https://attacker.tld/x-1.0.tar.gz>x<a y=\""
        )
        .is_err());
    }

    #[test]
    fn validate_rejects_traversal_and_separators() {
        for name in [
            "../../etc/passwd-1.0.tar.gz",
            "..-1.0.tar.gz",
            "sub/dir-1.0.tar.gz",
            ".hidden-1.0.tar.gz",
        ] {
            assert!(
                validate_distribution_filename(name).is_err(),
                "accepted: {name}"
            );
        }
    }

    #[test]
    fn validate_rejects_an_unparseable_filename() {
        assert!(validate_distribution_filename("notapackage.exe").is_err());
        assert!(
            validate_distribution_filename(&format!("{}-1.0.tar.gz", "a".repeat(300))).is_err()
        );
    }
}
