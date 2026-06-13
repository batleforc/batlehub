pub mod publish;
pub mod simple;

pub use publish::pypi_publish;
pub use simple::{pypi_file_download, pypi_simple_package, pypi_simple_root};

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
}
