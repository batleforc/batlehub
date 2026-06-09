use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_npm_deps(data: &Bytes) -> Vec<SbomDependency> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    let Ok(entries) = archive.entries() else {
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
                return vec![];
            }
            return parse_npm_package_json(&content);
        }
    }
    vec![]
}

fn parse_npm_package_json(content: &str) -> Vec<SbomDependency> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(content) else {
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
}
