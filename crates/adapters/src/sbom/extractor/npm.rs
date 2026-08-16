use batlehub_core::ports::{ExtractedManifest, SbomDependency};
use bytes::Bytes;

pub(super) fn extract_npm_manifest(data: &Bytes) -> ExtractedManifest {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    let Ok(entries) = archive.entries() else {
        tracing::warn!("sbom: failed to parse npm manifest, treating as no dependencies");
        return ExtractedManifest::default();
    };

    for entry in entries.flatten() {
        let Ok(path) = entry.path() else { continue };
        if path.file_name().and_then(|n| n.to_str()) == Some("package.json") {
            // Only the top-level package.json (direct child of "package/")
            let depth = path.components().count();
            if depth != 2 {
                continue;
            }
            let mut reader = entry;
            let mut content = String::new();
            if reader.read_to_string(&mut content).is_err() {
                tracing::warn!("sbom: failed to parse npm manifest, treating as no dependencies");
                return ExtractedManifest::default();
            }
            return parse_npm_package_json(&content);
        }
    }
    ExtractedManifest::default()
}

fn parse_npm_package_json(content: &str) -> ExtractedManifest {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        tracing::warn!("sbom: failed to parse npm manifest, treating as no dependencies");
        return ExtractedManifest::default();
    };

    let mut deps = Vec::new();
    for key in &["dependencies", "peerDependencies"] {
        if let Some(obj) = val.get(key).and_then(|v| v.as_object()) {
            for (name, ver) in obj {
                deps.push(SbomDependency {
                    name: name.clone(),
                    version_req: ver.as_str().map(|s| s.to_owned()),
                    ecosystem: "npm".into(),
                });
            }
        }
    }

    ExtractedManifest {
        dependencies: deps,
        license: parse_npm_license(&val),
    }
}

/// `"license": "MIT"`, or the pre-2015 `{"type": "MIT", "url": …}` object.
///
/// The deprecated `"licenses": [{…}]` array is read too, joined with ` OR ` —
/// npm's own documented meaning for it is a choice between them, which is what
/// the SPDX operator says. Packages this old are still in every lockfile that
/// has not been regenerated, so leaving them unknown would silently exempt them
/// from the gate.
fn parse_npm_license(val: &serde_json::Value) -> Option<String> {
    if let Some(s) = val.get("license").and_then(|v| v.as_str()) {
        let s = s.trim();
        if !s.is_empty() {
            return Some(s.to_owned());
        }
    }
    if let Some(t) = val
        .get("license")
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str())
    {
        let t = t.trim();
        if !t.is_empty() {
            return Some(t.to_owned());
        }
    }
    let joined: Vec<String> = val
        .get("licenses")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    entry
                        .as_str()
                        .or_else(|| entry.get("type").and_then(|t| t.as_str()))
                })
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if joined.is_empty() {
        None
    } else {
        Some(joined.join(" OR "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_npm_package_json_basic() {
        let json = r#"{"dependencies":{"express":"4.0.0"},"peerDependencies":{"react":"18"}}"#;
        let deps = parse_npm_package_json(json).dependencies;
        assert_eq!(deps.len(), 2);
        let names: Vec<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"express"));
        assert!(names.contains(&"react"));
    }

    #[test]
    fn parse_npm_package_json_invalid_is_empty() {
        assert_eq!(
            parse_npm_package_json("not json"),
            ExtractedManifest::default()
        );
        // Valid JSON without dependency keys → no deps.
        assert!(parse_npm_package_json(r#"{"name":"x"}"#)
            .dependencies
            .is_empty());
    }

    #[test]
    fn parse_npm_license_plain_string() {
        let m = parse_npm_package_json(r#"{"license":"MIT"}"#);
        assert_eq!(m.license.as_deref(), Some("MIT"));
    }

    /// The pre-2015 object form is still in published packages.
    #[test]
    fn parse_npm_license_object_form() {
        let m = parse_npm_package_json(r#"{"license":{"type":"ISC","url":"http://x"}}"#);
        assert_eq!(m.license.as_deref(), Some("ISC"));
    }

    /// The deprecated array means "any of these", which is SPDX `OR`.
    #[test]
    fn parse_npm_license_deprecated_array_joins_with_or() {
        let m = parse_npm_package_json(r#"{"licenses":[{"type":"MIT"},{"type":"GPL-3.0"}]}"#);
        assert_eq!(m.license.as_deref(), Some("MIT OR GPL-3.0"));
    }

    #[test]
    fn parse_npm_license_absent_is_none() {
        assert_eq!(parse_npm_package_json(r#"{"name":"x"}"#).license, None);
        assert_eq!(parse_npm_package_json(r#"{"license":"  "}"#).license, None);
    }

    /// Build a gzipped npm-style tarball containing `package/package.json`.
    fn npm_tgz(package_json: &[u8]) -> Bytes {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let mut header = tar::Header::new_gnu();
            header.set_size(package_json.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "package/package.json", package_json)
                .unwrap();
            builder.finish().unwrap();
        }
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_buf).unwrap();
        Bytes::from(gz.finish().unwrap())
    }

    #[test]
    fn extract_npm_deps_reads_top_level_package_json() {
        let data = npm_tgz(br#"{"license":"MIT","dependencies":{"express":"4.0.0"}}"#);
        let manifest = extract_npm_manifest(&data);
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(manifest.dependencies[0].name, "express");
        assert_eq!(manifest.dependencies[0].ecosystem, "npm");
        assert_eq!(
            manifest.dependencies[0].version_req.as_deref(),
            Some("4.0.0")
        );
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn extract_npm_deps_on_non_gzip_is_empty() {
        assert_eq!(
            extract_npm_manifest(&Bytes::from_static(b"not a gzip stream")),
            ExtractedManifest::default()
        );
    }
}
