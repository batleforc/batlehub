use batlehub_core::ports::{ExtractedManifest, SbomDependency};
use bytes::Bytes;

pub(super) fn extract_pypi_manifest(data: &Bytes) -> ExtractedManifest {
    // Try wheel (zip) first, then sdist (tar.gz).
    extract_pypi_wheel(data)
        .or_else(|| extract_pypi_sdist(data))
        .unwrap_or_default()
}

fn extract_pypi_wheel(data: &Bytes) -> Option<ExtractedManifest> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let Ok(mut archive) = ZipArchive::new(cursor) else {
        tracing::warn!("sbom: failed to parse pypi wheel manifest, treating as no dependencies");
        return None;
    };

    for i in 0..archive.len() {
        let Ok(mut file) = archive.by_index(i) else {
            continue;
        };
        let name = file.name().to_owned();
        if name.ends_with(".dist-info/METADATA") {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_err() {
                tracing::warn!(
                    "sbom: failed to parse pypi wheel manifest, treating as no dependencies"
                );
                return None;
            }
            return Some(parse_pep_metadata(&content));
        }
    }
    None
}

fn extract_pypi_sdist(data: &Bytes) -> Option<ExtractedManifest> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    let Ok(entries) = archive.entries() else {
        tracing::warn!("sbom: failed to parse pypi sdist manifest, treating as no dependencies");
        return None;
    };

    for entry in entries.flatten() {
        let Ok(path) = entry.path() else { continue };
        let path = path.into_owned();
        let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if fname == "PKG-INFO" || fname == "METADATA" {
            let mut reader = entry;
            let mut content = String::new();
            if reader.read_to_string(&mut content).is_ok() {
                return Some(parse_pep_metadata(&content));
            }
        }
    }
    None
}

fn parse_pep_metadata(content: &str) -> ExtractedManifest {
    let dependencies = content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix("Requires-Dist:").map(|rest| {
                // "Requires-Dist: requests >=2.0" or "requests"
                let dep = rest.trim().split(';').next().unwrap_or(rest.trim());
                let mut parts = dep.splitn(2, ' ');
                let name = parts.next().unwrap_or("").trim().to_owned();
                let ver = parts.next().map(|v| v.trim().to_owned());
                SbomDependency {
                    name,
                    version_req: ver.filter(|v| !v.is_empty()),
                    ecosystem: "pypi".into(),
                }
            })
        })
        .filter(|d| !d.name.is_empty())
        .collect();

    ExtractedManifest {
        dependencies,
        license: parse_pep_license(content),
    }
}

/// PEP 639's `License-Expression` first, then the legacy `License`, then the
/// `License ::` trove classifier.
///
/// The order is the specification's own precedence, and it matters: a package
/// mid-migration carries both, with `License` holding free text like
/// `"BSD-3-Clause or later, see LICENSE"` while `License-Expression` holds the
/// SPDX id. Reading the legacy field first would hand the gate prose.
///
/// The classifier fallback keeps only the trailing segment — `License :: OSI
/// Approved :: MIT License` is a taxonomy path, not an expression, so the
/// stored value is `MIT License`, which the gate's normalisation handles.
fn parse_pep_license(content: &str) -> Option<String> {
    let field = |name: &str| {
        content.lines().find_map(|line| {
            line.trim()
                .strip_prefix(name)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        })
    };

    if let Some(v) = field("License-Expression:") {
        return Some(v);
    }
    if let Some(v) = field("License:") {
        // A multi-line `License:` continuation is the licence *text* inlined,
        // which is not an expression. One line is all this reads.
        return Some(v);
    }

    let classifiers: Vec<String> = content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Classifier:"))
        .map(str::trim)
        .filter(|c| c.starts_with("License ::"))
        .filter_map(|c| c.rsplit("::").next())
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "OSI Approved")
        .map(str::to_owned)
        .collect();

    if classifiers.is_empty() {
        None
    } else {
        Some(classifiers.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pep_metadata_basic() {
        let metadata =
            "Name: requests\nVersion: 2.31.0\nRequires-Dist: urllib3 >=1.21\nRequires-Dist: certifi\n";
        let deps = parse_pep_metadata(metadata).dependencies;
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "urllib3");
        assert_eq!(deps[0].version_req.as_deref(), Some(">=1.21"));
        assert_eq!(deps[1].name, "certifi");
        assert!(deps[1].version_req.is_none());
    }

    #[test]
    fn parse_pep_metadata_strips_environment_markers() {
        let deps = parse_pep_metadata("Requires-Dist: pytest >=7 ; extra == 'test'\n").dependencies;
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "pytest");
        assert_eq!(deps[0].version_req.as_deref(), Some(">=7"));
        assert_eq!(deps[0].ecosystem, "pypi");
    }

    #[test]
    fn parse_pep_license_prefers_expression_over_legacy_field() {
        let metadata =
            "License: BSD-3-Clause or later, see LICENSE\nLicense-Expression: BSD-3-Clause\n";
        assert_eq!(parse_pep_license(metadata).as_deref(), Some("BSD-3-Clause"));
    }

    #[test]
    fn parse_pep_license_falls_back_to_legacy_field() {
        assert_eq!(parse_pep_license("License: MIT\n").as_deref(), Some("MIT"));
    }

    /// `License :: OSI Approved :: MIT License` is a taxonomy path; only the
    /// leaf is a licence name, and the `OSI Approved` interior node is not one.
    #[test]
    fn parse_pep_license_falls_back_to_classifier_leaf() {
        let metadata = "Classifier: Programming Language :: Python\nClassifier: License :: OSI Approved :: MIT License\n";
        assert_eq!(parse_pep_license(metadata).as_deref(), Some("MIT License"));
    }

    #[test]
    fn parse_pep_license_ignores_license_file_field() {
        // `License-File` names a file, like Cargo's `license-file`.
        assert_eq!(parse_pep_license("License-File: LICENSE\n"), None);
    }

    #[test]
    fn parse_pep_license_absent_is_none() {
        assert_eq!(parse_pep_license("Name: x\nVersion: 1.0\n"), None);
    }

    #[test]
    fn extract_pypi_deps_reads_wheel_metadata() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            zw.start_file(
                "requests-2.31.0.dist-info/METADATA",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zw.write_all(b"Requires-Dist: urllib3 >=1.21\n").unwrap();
            zw.finish().unwrap();
        }
        let deps = extract_pypi_manifest(&Bytes::from(buf)).dependencies;
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "urllib3");
        assert_eq!(deps[0].ecosystem, "pypi");
    }

    #[test]
    fn extract_pypi_deps_reads_sdist_pkg_info() {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let pkg_info: &[u8] = b"Requires-Dist: certifi\n";
        let mut tar_buf = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_buf);
            let mut h = tar::Header::new_gnu();
            h.set_size(pkg_info.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, "requests-2.31.0/PKG-INFO", pkg_info)
                .unwrap();
            b.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_buf).unwrap();
        let deps = extract_pypi_manifest(&Bytes::from(gz.finish().unwrap())).dependencies;
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "certifi");
    }

    #[test]
    fn extract_pypi_deps_on_garbage_is_empty() {
        assert_eq!(
            extract_pypi_manifest(&Bytes::from_static(b"not an archive")),
            ExtractedManifest::default()
        );
    }
}
