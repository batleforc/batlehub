//! What markup a README is, decided from what the *protocol* says.
//!
//! Never from an upstream's `Content-Type`: an upstream declaring `text/html`
//! for a document its own protocol says is markdown must not be able to choose
//! which renderer path runs (RFC 0007 §7.4). The two inputs here are a filename
//! inside an archive, and the content-type field a protocol defines for the
//! purpose — PyPI's `description_content_type`, which is part of the package
//! metadata the publisher wrote, not a transport header.

use crate::entities::ReadmeFormat;

/// The format a README filename implies.
///
/// An unknown or absent extension is [`ReadmeFormat::Plain`], which escapes the
/// text into a `<pre>` rather than parsing it. That is the safe direction: a
/// misidentified plain file renders as ugly text, a misidentified markup file
/// renders as markup.
pub fn format_from_filename(path: &str) -> ReadmeFormat {
    let lower = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    match lower.rsplit_once('.') {
        Some((_, "md" | "markdown" | "mkd" | "mdown")) => ReadmeFormat::Markdown,
        Some((_, "rst")) => ReadmeFormat::Rst,
        Some((_, "html" | "htm")) => ReadmeFormat::Html,
        // `README` with no extension is the oldest convention there is, and it
        // is prose. Not markdown: nothing declared it as such.
        _ => ReadmeFormat::Plain,
    }
}

/// Whether a filename inside an archive looks like the package's README.
///
/// A convention match, deliberately narrow: `README`, `README.md`, `readme.rst`
/// and so on, at whatever path the caller is scanning. It does **not** match
/// `READMExample.md` or `guide/README-for-contributors.md`, because a README
/// panel showing the wrong document is worse than one showing none.
pub fn is_readme_filename(path: &str) -> bool {
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = base.split_once('.').map_or(base, |(s, _)| s);
    stem.eq_ignore_ascii_case("readme")
}

/// The format a protocol-declared content type implies.
///
/// PyPI's `description_content_type` is the case this exists for: it is written
/// by the publisher in the package metadata (PEP 566) and names the markup of
/// `info.description`. Parameters are ignored — `text/markdown; charset=UTF-8;
/// variant=GFM` is markdown.
///
/// An unrecognised or absent value is [`ReadmeFormat::Plain`], which is what
/// PEP 566 itself specifies as the default.
pub fn format_from_content_type(content_type: Option<&str>) -> ReadmeFormat {
    let Some(raw) = content_type else {
        return ReadmeFormat::Plain;
    };
    let base = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "text/markdown" => ReadmeFormat::Markdown,
        "text/x-rst" => ReadmeFormat::Rst,
        "text/html" => ReadmeFormat::Html,
        _ => ReadmeFormat::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_map_to_their_markup() {
        assert_eq!(format_from_filename("README.md"), ReadmeFormat::Markdown);
        assert_eq!(
            format_from_filename("docs/readme.MARKDOWN"),
            ReadmeFormat::Markdown
        );
        assert_eq!(format_from_filename("README.rst"), ReadmeFormat::Rst);
        assert_eq!(format_from_filename("README.html"), ReadmeFormat::Html);
        assert_eq!(format_from_filename("README.htm"), ReadmeFormat::Html);
    }

    /// Unknown and absent extensions both escape rather than parse. A README
    /// with no extension is prose; nothing declared it as markdown.
    #[test]
    fn an_undeclared_format_is_escaped_not_guessed() {
        assert_eq!(format_from_filename("README"), ReadmeFormat::Plain);
        assert_eq!(format_from_filename("README.txt"), ReadmeFormat::Plain);
        assert_eq!(format_from_filename("README.adoc"), ReadmeFormat::Plain);
        assert_eq!(format_from_filename(""), ReadmeFormat::Plain);
    }

    #[test]
    fn the_readme_convention_is_matched_narrowly() {
        assert!(is_readme_filename("README"));
        assert!(is_readme_filename("readme.md"));
        assert!(is_readme_filename("ReadMe.RST"));
        assert!(is_readme_filename("pkg-1.0.0/README.md"));
        // A README panel showing the wrong document is worse than none.
        assert!(!is_readme_filename("READMExample.md"));
        assert!(!is_readme_filename("README-for-contributors.md"));
        assert!(!is_readme_filename("CHANGELOG.md"));
        assert!(!is_readme_filename(""));
    }

    #[test]
    fn declared_content_types_map_with_their_parameters_ignored() {
        assert_eq!(
            format_from_content_type(Some("text/markdown")),
            ReadmeFormat::Markdown
        );
        assert_eq!(
            format_from_content_type(Some("text/markdown; charset=UTF-8; variant=GFM")),
            ReadmeFormat::Markdown
        );
        assert_eq!(
            format_from_content_type(Some("TEXT/X-RST")),
            ReadmeFormat::Rst
        );
        assert_eq!(
            format_from_content_type(Some("text/html")),
            ReadmeFormat::Html
        );
    }

    /// PEP 566 says an absent `description_content_type` means plain text, and
    /// an unrecognised one must not be guessed into a renderer.
    #[test]
    fn an_absent_or_unknown_content_type_is_plain() {
        assert_eq!(format_from_content_type(None), ReadmeFormat::Plain);
        assert_eq!(
            format_from_content_type(Some("text/x-asciidoc")),
            ReadmeFormat::Plain
        );
        assert_eq!(format_from_content_type(Some("")), ReadmeFormat::Plain);
    }
}
