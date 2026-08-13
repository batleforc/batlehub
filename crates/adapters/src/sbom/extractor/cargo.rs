use batlehub_core::ports::{ExtractedManifest, SbomDependency};
use bytes::Bytes;

pub(super) fn extract_cargo_manifest(data: &Bytes) -> ExtractedManifest {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    let Ok(entries) = archive.entries() else {
        tracing::warn!("sbom: failed to parse cargo manifest, treating as no dependencies");
        return ExtractedManifest::default();
    };

    for entry in entries.flatten() {
        let Ok(path) = entry.path() else { continue };
        if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
            let mut reader = entry;
            let mut content = String::new();
            if reader.read_to_string(&mut content).is_err() {
                tracing::warn!("sbom: failed to parse cargo manifest, treating as no dependencies");
                return ExtractedManifest::default();
            }
            return ExtractedManifest {
                dependencies: parse_cargo_toml_deps(&content),
                license: parse_cargo_toml_license(&content),
            };
        }
    }
    ExtractedManifest::default()
}

/// `[package] license = "MIT OR Apache-2.0"`.
///
/// `license-file` is deliberately *not* a fallback: it names a file whose
/// contents are the licence text, so the value would be `LICENSE` — a filename
/// masquerading as an SPDX expression, which a gate would then compare against
/// its allow list and never match. A crate that only sets `license-file` is
/// correctly reported as unknown.
fn parse_cargo_toml_license(content: &str) -> Option<String> {
    let mut in_package = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("license") {
            let rest = rest.trim();
            let Some(value) = rest.strip_prefix('=') else {
                continue; // `license-file = …` and anything else starting "license"
            };
            let value = value.trim().trim_matches('"').trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn parse_version_from_toml_rest(rest: &str) -> String {
    if rest.starts_with('"') {
        rest.trim_matches('"').to_owned()
    } else if let Some(start) = rest.find("version = \"") {
        let after = &rest[start + 11..];
        after
            .find('"')
            .map(|end| after[..end].to_owned())
            .unwrap_or_default()
    } else {
        String::new()
    }
}

fn parse_dep_entry(trimmed: &str) -> Option<SbomDependency> {
    let (name, rest) = trimmed.split_once('=')?;
    let name = name.trim().trim_matches('"');
    if name.is_empty() || name.starts_with('#') {
        return None;
    }
    let version = parse_version_from_toml_rest(rest.trim());
    Some(SbomDependency {
        name: name.to_owned(),
        version_req: if version.is_empty() {
            None
        } else {
            Some(version)
        },
        ecosystem: "cargo".into(),
    })
}

fn parse_cargo_toml_deps(content: &str) -> Vec<SbomDependency> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut in_dev_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[dependencies]" {
            in_deps = true;
            in_dev_deps = false;
            continue;
        }
        if trimmed == "[dev-dependencies]" || trimmed == "[build-dependencies]" {
            in_deps = false;
            in_dev_deps = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_deps = false;
            in_dev_deps = false;
            continue;
        }
        if !in_deps && !in_dev_deps {
            continue;
        }
        if let Some(dep) = parse_dep_entry(trimmed) {
            deps.push(dep);
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_toml_basic() {
        let toml = r#"
[dependencies]
serde = "1.0"
tokio = { version = "1.0", features = ["full"] }
"#;
        let deps = parse_cargo_toml_deps(toml);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "serde");
        assert_eq!(deps[0].version_req.as_deref(), Some("1.0"));
        assert_eq!(deps[1].name, "tokio");
        assert_eq!(deps[1].version_req.as_deref(), Some("1.0"));
    }

    #[test]
    fn parse_cargo_toml_license_from_package_section() {
        let toml = "[package]\nname = \"x\"\nlicense = \"MIT OR Apache-2.0\"\n";
        assert_eq!(
            parse_cargo_toml_license(toml).as_deref(),
            Some("MIT OR Apache-2.0")
        );
    }

    /// `license-file` names a file, not an expression. Reporting `LICENSE` as
    /// the licence would give the gate a value that matches no allow list and
    /// reads as a real declaration.
    #[test]
    fn parse_cargo_toml_license_ignores_license_file() {
        let toml = "[package]\nlicense-file = \"LICENSE\"\n";
        assert_eq!(parse_cargo_toml_license(toml), None);
    }

    /// A `license` key under `[dependencies]` (or any other table) is not the
    /// crate's own declaration.
    #[test]
    fn parse_cargo_toml_license_only_reads_package_table() {
        let toml = "[dependencies]\nlicense = \"GPL-3.0\"\n";
        assert_eq!(parse_cargo_toml_license(toml), None);
    }

    #[test]
    fn parse_cargo_toml_license_absent_is_none() {
        assert_eq!(parse_cargo_toml_license("[package]\nname = \"x\"\n"), None);
    }
}
