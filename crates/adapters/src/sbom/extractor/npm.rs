use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_npm_deps(data: &Bytes) -> Vec<SbomDependency> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    let Ok(entries) = archive.entries() else {
        tracing::warn!("sbom: failed to parse npm manifest, treating as no dependencies");
        return vec![];
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
                return vec![];
            }
            return parse_npm_package_json(&content);
        }
    }
    vec![]
}

fn parse_npm_package_json(content: &str) -> Vec<SbomDependency> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
        tracing::warn!("sbom: failed to parse npm manifest, treating as no dependencies");
        return vec![];
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
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_npm_package_json_basic() {
        let json = r#"{"dependencies":{"express":"4.0.0"},"peerDependencies":{"react":"18"}}"#;
        let deps = parse_npm_package_json(json);
        assert_eq!(deps.len(), 2);
        let names: Vec<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"express"));
        assert!(names.contains(&"react"));
    }

    #[test]
    fn parse_npm_package_json_invalid_is_empty() {
        assert!(parse_npm_package_json("not json").is_empty());
        // Valid JSON without dependency keys → no deps.
        assert!(parse_npm_package_json(r#"{"name":"x"}"#).is_empty());
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
        let data = npm_tgz(br#"{"dependencies":{"express":"4.0.0"}}"#);
        let deps = extract_npm_deps(&data);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "express");
        assert_eq!(deps[0].ecosystem, "npm");
        assert_eq!(deps[0].version_req.as_deref(), Some("4.0.0"));
    }

    #[test]
    fn extract_npm_deps_on_non_gzip_is_empty() {
        assert!(extract_npm_deps(&Bytes::from_static(b"not a gzip stream")).is_empty());
    }
}
