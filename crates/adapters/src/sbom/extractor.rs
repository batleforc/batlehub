use bytes::Bytes;

use batlehub_core::ports::SbomDependency;

/// Archive-based SBOM dependency extractor.
///
/// Parses dependency manifests embedded in package archives.
/// Requires the `sbom` feature (which enables flate2, tar, zip, quick-xml).
pub struct ArchiveSbomExtractor;

impl batlehub_core::ports::SbomExtractor for ArchiveSbomExtractor {
    fn extract(&self, data: &Bytes, registry_type: &str) -> Vec<SbomDependency> {
        match registry_type {
            "cargo" => extract_cargo_deps(data),
            "npm" => extract_npm_deps(data),
            "maven" => extract_maven_deps(data),
            "pypi" => extract_pypi_deps(data),
            "nuget" => extract_nuget_deps(data),
            _ => vec![],
        }
    }
}

// ── Cargo (.crate = .tar.gz) ──────────────────────────────────────────────────

fn extract_cargo_deps(data: &Bytes) -> Vec<SbomDependency> {
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
        if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
            let mut reader = entry;
            let mut content = String::new();
            if reader.read_to_string(&mut content).is_err() {
                return vec![];
            }
            return parse_cargo_toml_deps(&content);
        }
    }
    vec![]
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
        version_req: if version.is_empty() { None } else { Some(version) },
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

// ── npm (.tgz = .tar.gz) ─────────────────────────────────────────────────────

fn extract_npm_deps(data: &Bytes) -> Vec<SbomDependency> {
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

// ── Maven (.jar = zip with pom.xml inside) ────────────────────────────────────

fn extract_maven_deps(data: &Bytes) -> Vec<SbomDependency> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let Ok(mut archive) = ZipArchive::new(cursor) else {
        return vec![];
    };

    for i in 0..archive.len() {
        let Ok(mut file) = archive.by_index(i) else {
            continue;
        };
        let name = file.name().to_owned();
        if name.ends_with("pom.xml") {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_err() {
                return vec![];
            }
            return parse_maven_pom(&content);
        }
    }
    vec![]
}

fn decode_xml_text(e: &quick_xml::events::BytesText) -> String {
    match e.decode() {
        Ok(raw) => quick_xml::escape::unescape(&raw)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| raw.into_owned()),
        Err(_) => String::new(),
    }
}

fn finalize_maven_dependency(group: &str, artifact: &str, version: &str) -> Option<SbomDependency> {
    if artifact.is_empty() {
        return None;
    }
    let name = if group.is_empty() {
        artifact.to_owned()
    } else {
        format!("{group}:{artifact}")
    };
    Some(SbomDependency {
        name,
        version_req: if version.is_empty() {
            None
        } else {
            Some(version.to_owned())
        },
        ecosystem: "maven".into(),
    })
}

fn parse_maven_pom(content: &str) -> Vec<SbomDependency> {
    use quick_xml::{events::Event, Reader};

    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut deps = Vec::new();
    let mut in_dependency = 0u32;
    let mut current_group = String::new();
    let mut current_artifact = String::new();
    let mut current_version = String::new();
    let mut capture_field: Option<&'static str> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let ln = e.local_name();
                let local = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                match local {
                    "dependency" => in_dependency += 1,
                    "groupId" if in_dependency > 0 => capture_field = Some("groupId"),
                    "artifactId" if in_dependency > 0 => capture_field = Some("artifactId"),
                    "version" if in_dependency > 0 => capture_field = Some("version"),
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Some(field) = capture_field.take() {
                    let text = decode_xml_text(e);
                    match field {
                        "groupId" => current_group = text,
                        "artifactId" => current_artifact = text,
                        "version" => current_version = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let local = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                if local == "dependency" && in_dependency > 0 {
                    in_dependency -= 1;
                    if let Some(dep) = finalize_maven_dependency(
                        &current_group,
                        &current_artifact,
                        &current_version,
                    ) {
                        deps.push(dep);
                    }
                    current_group.clear();
                    current_artifact.clear();
                    current_version.clear();
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    deps
}

// ── PyPI (.whl = zip with METADATA; .tar.gz with PKG-INFO) ───────────────────

fn extract_pypi_deps(data: &Bytes) -> Vec<SbomDependency> {
    // Try wheel (zip) first, then sdist (tar.gz).
    extract_pypi_wheel(data)
        .or_else(|| extract_pypi_sdist(data))
        .unwrap_or_default()
}

fn extract_pypi_wheel(data: &Bytes) -> Option<Vec<SbomDependency>> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let mut archive = ZipArchive::new(cursor).ok()?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).ok()?;
        let name = file.name().to_owned();
        if name.ends_with(".dist-info/METADATA") {
            let mut content = String::new();
            file.read_to_string(&mut content).ok()?;
            return Some(parse_pep_metadata(&content));
        }
    }
    None
}

fn extract_pypi_sdist(data: &Bytes) -> Option<Vec<SbomDependency>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    let gz = GzDecoder::new(data.as_ref());
    let mut archive = Archive::new(gz);

    for entry in archive.entries().ok()?.flatten() {
        let path = entry.path().ok()?.into_owned();
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

fn parse_pep_metadata(content: &str) -> Vec<SbomDependency> {
    content
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
        .collect()
}

// ── NuGet (.nupkg = zip with *.nuspec) ───────────────────────────────────────

fn extract_nuget_deps(data: &Bytes) -> Vec<SbomDependency> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let Ok(mut archive) = ZipArchive::new(cursor) else {
        return vec![];
    };

    for i in 0..archive.len() {
        let Ok(mut file) = archive.by_index(i) else {
            continue;
        };
        let name = file.name().to_owned();
        if name.ends_with(".nuspec") {
            let mut content = String::new();
            if file.read_to_string(&mut content).is_err() {
                return vec![];
            }
            return parse_nuspec_deps(&content);
        }
    }
    vec![]
}

fn parse_nuget_dep_from_empty<'a>(
    e: &quick_xml::events::BytesStart<'a>,
    decoder: quick_xml::Decoder,
) -> Option<SbomDependency> {
    let mut id = String::new();
    let mut version = String::new();
    for attr in e.attributes().flatten() {
        let kn = attr.key.local_name();
        let key = std::str::from_utf8(kn.as_ref()).unwrap_or("");
        let val = attr
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
            .map(|v| v.into_owned())
            .unwrap_or_default();
        match key {
            "id" => id = val,
            "version" => version = val,
            _ => {}
        }
    }
    if id.is_empty() {
        return None;
    }
    Some(SbomDependency {
        name: id,
        version_req: if version.is_empty() {
            None
        } else {
            Some(version)
        },
        ecosystem: "nuget".into(),
    })
}

fn parse_nuspec_deps(content: &str) -> Vec<SbomDependency> {
    use quick_xml::{events::Event, Reader};

    let mut reader = Reader::from_str(content);
    reader.config_mut().trim_text(true);
    let mut deps = Vec::new();

    // <dependency> elements in .nuspec are always self-closing:
    //   <dependency id="Newtonsoft.Json" version="[13.0,)" />
    loop {
        match reader.read_event() {
            Ok(Event::Empty(ref e)) => {
                let ln = e.local_name();
                let local = std::str::from_utf8(ln.as_ref()).unwrap_or("");
                if local == "dependency" {
                    if let Some(dep) = parse_nuget_dep_from_empty(e, reader.decoder()) {
                        deps.push(dep);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    deps
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use batlehub_core::ports::SbomExtractor;

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
    fn parse_npm_package_json_basic() {
        let json = r#"{"dependencies":{"express":"4.0.0"},"peerDependencies":{"react":"18"}}"#;
        let deps = parse_npm_package_json(json);
        assert_eq!(deps.len(), 2);
        let names: Vec<_> = deps.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"express"));
        assert!(names.contains(&"react"));
    }

    #[test]
    fn parse_pep_metadata_basic() {
        let metadata = "Name: requests\nVersion: 2.31.0\nRequires-Dist: urllib3 >=1.21\nRequires-Dist: certifi\n";
        let deps = parse_pep_metadata(metadata);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "urllib3");
        assert_eq!(deps[0].version_req.as_deref(), Some(">=1.21"));
        assert_eq!(deps[1].name, "certifi");
        assert!(deps[1].version_req.is_none());
    }

    #[test]
    fn extract_returns_empty_for_unknown_type() {
        let extractor = ArchiveSbomExtractor;
        let data = Bytes::from_static(b"not an archive");
        assert!(extractor.extract(&data, "unknown").is_empty());
    }

    #[test]
    fn parse_nuspec_deps_basic() {
        let nuspec = r#"<?xml version="1.0"?>
<package>
  <metadata>
    <id>MyLib</id>
    <version>1.0.0</version>
    <dependencies>
      <group targetFramework="net6.0">
        <dependency id="Newtonsoft.Json" version="[13.0,)" />
        <dependency id="Serilog" version="2.12.0" />
      </group>
    </dependencies>
  </metadata>
</package>"#;
        let deps = parse_nuspec_deps(nuspec);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "Newtonsoft.Json");
        assert_eq!(deps[0].version_req.as_deref(), Some("[13.0,)"));
        assert_eq!(deps[0].ecosystem, "nuget");
        assert_eq!(deps[1].name, "Serilog");
        assert_eq!(deps[1].version_req.as_deref(), Some("2.12.0"));
    }

    #[test]
    fn parse_nuspec_deps_no_version() {
        let nuspec = r#"<package><metadata><dependencies>
          <dependency id="SomeLib" />
        </dependencies></metadata></package>"#;
        let deps = parse_nuspec_deps(nuspec);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "SomeLib");
        assert!(deps[0].version_req.is_none());
    }

    #[test]
    fn parse_nuspec_deps_empty_deps() {
        let nuspec = r#"<package><metadata><id>Foo</id></metadata></package>"#;
        let deps = parse_nuspec_deps(nuspec);
        assert!(deps.is_empty());
    }

    fn make_nupkg_with_nuspec(nuspec: &str) -> Bytes {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        zip.start_file("mylib.nuspec", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(nuspec.as_bytes()).unwrap();
        zip.finish().unwrap();
        Bytes::from(buf)
    }

    #[test]
    fn extract_nuget_deps_from_nupkg() {
        let nuspec = r#"<package><metadata><dependencies>
          <dependency id="Newtonsoft.Json" version="13.0.0" />
        </dependencies></metadata></package>"#;
        let data = make_nupkg_with_nuspec(nuspec);
        let extractor = ArchiveSbomExtractor;
        let deps = extractor.extract(&data, "nuget");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "Newtonsoft.Json");
    }

    #[test]
    fn extract_nuget_deps_invalid_zip() {
        let extractor = ArchiveSbomExtractor;
        let deps = extractor.extract(&Bytes::from_static(b"not a zip"), "nuget");
        assert!(deps.is_empty());
    }
}
