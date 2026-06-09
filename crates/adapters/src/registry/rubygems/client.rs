use super::{models, CoreError};
use models::GemMetadata;

// ── Local publish helpers ─────────────────────────────────────────────────────

/// Parse a `.gem` file (TAR archive containing `metadata.gz`) and extract gem metadata.
///
/// The `.gem` format is a TAR archive with:
/// - `metadata.gz` — gzip-compressed YAML gem specification
/// - `data.tar.gz` — the gem's actual files
#[cfg(feature = "local-registry")]
pub fn parse_gem_bytes(data: &[u8]) -> Result<GemMetadata, CoreError> {
    use std::io::{Cursor, Read};

    let cursor = Cursor::new(data);
    let mut archive = tar::Archive::new(cursor);

    let mut metadata_bytes: Option<Vec<u8>> = None;
    for entry in archive
        .entries()
        .map_err(|e| CoreError::Registry(format!("rubygems: read gem tar: {e}")))?
    {
        let mut entry =
            entry.map_err(|e| CoreError::Registry(format!("rubygems: gem entry: {e}")))?;
        let is_metadata = entry
            .path()
            .map(|p| p.as_os_str() == "metadata.gz")
            .unwrap_or(false);
        if is_metadata {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| CoreError::Registry(format!("rubygems: read metadata.gz: {e}")))?;
            metadata_bytes = Some(buf);
            break;
        }
    }

    let compressed = metadata_bytes.ok_or_else(|| {
        CoreError::Registry("rubygems: metadata.gz not found in .gem archive".to_owned())
    })?;

    let mut decoder = flate2::read::GzDecoder::new(compressed.as_slice());
    let mut yaml = String::new();
    decoder
        .read_to_string(&mut yaml)
        .map_err(|e| CoreError::Registry(format!("rubygems: decompress metadata.gz: {e}")))?;

    parse_gem_yaml(&yaml)
}

fn extract_yaml_value<'a>(yaml: &'a str, key: &str) -> Option<&'a str> {
    for line in yaml.lines() {
        let trimmed = line.trim_start_matches(' ');
        if let Some(rest) = trimmed.strip_prefix(key) {
            let v = rest.trim();
            if v.starts_with('!') {
                return None;
            }
            return Some(strip_yaml_quotes(v));
        }
    }
    None
}

fn strip_yaml_quotes(s: &str) -> &str {
    if s.len() >= 2
        && ((s.starts_with('\'') && s.ends_with('\'')) || (s.starts_with('"') && s.ends_with('"')))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Extract the gem version from the nested Gem::Version YAML block:
///   version: !ruby/object:Gem::Version
///     version: '1.0.0'
fn extract_gem_version(yaml: &str) -> Option<String> {
    let mut after_version_key = false;
    for line in yaml.lines() {
        if line.starts_with("version:") {
            after_version_key = true;
            continue;
        }
        if !after_version_key {
            continue;
        }
        let trimmed = line.trim_start_matches(' ');
        if let Some(rest) = trimmed.strip_prefix("version: ") {
            let v = rest.trim();
            if !v.starts_with('!') {
                return Some(strip_yaml_quotes(v).to_owned());
            }
        }
        if !line.starts_with(' ') {
            after_version_key = false;
        }
    }
    None
}

pub(super) fn parse_gem_yaml(yaml: &str) -> Result<GemMetadata, CoreError> {
    let name = extract_yaml_value(yaml, "name: ")
        .ok_or_else(|| CoreError::Registry("rubygems: gem name not found in metadata".to_owned()))?
        .to_owned();

    let version = extract_gem_version(yaml).ok_or_else(|| {
        CoreError::Registry("rubygems: gem version not found in metadata".to_owned())
    })?;

    let platform = extract_yaml_value(yaml, "platform: ")
        .unwrap_or("ruby")
        .to_owned();
    let summary = extract_yaml_value(yaml, "summary: ").map(str::to_owned);

    let mut authors = Vec::new();
    let mut in_authors = false;
    for line in yaml.lines() {
        if line == "authors:" || line.starts_with("authors:") {
            in_authors = true;
            continue;
        }
        if in_authors {
            let trimmed = line.trim_start_matches(' ');
            if let Some(author) = trimmed.strip_prefix("- ") {
                authors.push(strip_yaml_quotes(author.trim()).to_owned());
            } else {
                in_authors = false;
            }
        }
    }

    Ok(GemMetadata {
        name,
        version,
        platform,
        summary,
        authors,
    })
}

/// Split a gem filename stem (without `.gem`) into `(name, version)`.
///
/// Gem filenames follow `{name}-{version}` where version starts with a digit.
/// For multi-hyphen gem names like `json-jwt`, the split point is at the first
/// `-` that is immediately followed by a digit.
pub fn split_gem_stem(stem: &str) -> Option<(&str, &str)> {
    let bytes = stem.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            return Some((&stem[..i], &stem[i + 1..]));
        }
    }
    None
}
