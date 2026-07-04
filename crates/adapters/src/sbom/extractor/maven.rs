use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_maven_deps(data: &Bytes) -> Vec<SbomDependency> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data.as_ref());
    let Ok(mut archive) = ZipArchive::new(cursor) else {
        tracing::warn!("sbom: failed to parse maven manifest, treating as no dependencies");
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
                tracing::warn!("sbom: failed to parse maven manifest, treating as no dependencies");
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

fn apply_maven_start(local: &str, in_dep: &mut u32, capture: &mut Option<&'static str>) {
    match local {
        "dependency" => *in_dep += 1,
        "groupId" if *in_dep > 0 => *capture = Some("groupId"),
        "artifactId" if *in_dep > 0 => *capture = Some("artifactId"),
        "version" if *in_dep > 0 => *capture = Some("version"),
        _ => {}
    }
}

fn apply_maven_end(
    local: &str,
    in_dep: &mut u32,
    group: &mut String,
    artifact: &mut String,
    version: &mut String,
    deps: &mut Vec<SbomDependency>,
) {
    if local == "dependency" && *in_dep > 0 {
        *in_dep -= 1;
        if let Some(dep) = finalize_maven_dependency(group, artifact, version) {
            deps.push(dep);
        }
        group.clear();
        artifact.clear();
        version.clear();
    }
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
                apply_maven_start(local, &mut in_dependency, &mut capture_field);
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
                apply_maven_end(
                    local,
                    &mut in_dependency,
                    &mut current_group,
                    &mut current_artifact,
                    &mut current_version,
                    &mut deps,
                );
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pom(deps_xml: &str) -> String {
        format!(
            r#"<project>
                <groupId>com.example</groupId>
                <artifactId>app</artifactId>
                <version>9.9.9</version>
                <dependencies>{deps_xml}</dependencies>
            </project>"#
        )
    }

    #[test]
    fn parse_maven_pom_basic() {
        let xml = pom(r#"<dependency>
                <groupId>com.fasterxml.jackson.core</groupId>
                <artifactId>jackson-databind</artifactId>
                <version>2.15.0</version>
            </dependency>"#);
        let deps = parse_maven_pom(&xml);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "com.fasterxml.jackson.core:jackson-databind");
        assert_eq!(deps[0].version_req.as_deref(), Some("2.15.0"));
        assert_eq!(deps[0].ecosystem, "maven");
    }

    #[test]
    fn parse_maven_pom_no_group_id() {
        let xml = pom(r#"<dependency>
                <artifactId>standalone</artifactId>
                <version>1.0</version>
            </dependency>"#);
        let deps = parse_maven_pom(&xml);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "standalone");
    }

    #[test]
    fn parse_maven_pom_empty_artifact_id_skipped() {
        let xml = pom(r#"<dependency>
                <groupId>com.example</groupId>
                <version>1.0</version>
            </dependency>"#);
        let deps = parse_maven_pom(&xml);
        assert!(deps.is_empty());
    }

    #[test]
    fn parse_maven_pom_multiple_dependencies() {
        let xml = pom(r#"<dependency>
                <groupId>g1</groupId>
                <artifactId>a1</artifactId>
                <version>1.0</version>
            </dependency>
            <dependency>
                <groupId>g2</groupId>
                <artifactId>a2</artifactId>
                <version>2.0</version>
            </dependency>"#);
        let deps = parse_maven_pom(&xml);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "g1:a1");
        assert_eq!(deps[0].version_req.as_deref(), Some("1.0"));
        assert_eq!(deps[1].name, "g2:a2");
        assert_eq!(deps[1].version_req.as_deref(), Some("2.0"));
    }

    #[test]
    fn parse_maven_pom_ignores_project_level_version() {
        // The project-level <version> (9.9.9, outside <dependencies>) must not
        // leak into the dependency's version.
        let xml = pom(r#"<dependency>
                <groupId>g1</groupId>
                <artifactId>a1</artifactId>
            </dependency>"#);
        let deps = parse_maven_pom(&xml);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].version_req, None);
    }

    #[test]
    fn extract_maven_deps_non_zip_returns_empty() {
        let data = Bytes::from_static(b"not a zip archive");
        assert!(extract_maven_deps(&data).is_empty());
    }

    #[test]
    fn extract_maven_deps_from_jar_with_pom() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let xml = pom(r#"<dependency>
                <groupId>g</groupId>
                <artifactId>a</artifactId>
                <version>1.2.3</version>
            </dependency>"#);

        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            writer
                .start_file(
                    "META-INF/maven/com.example/app/pom.xml",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(xml.as_bytes()).unwrap();
            writer.finish().unwrap();
        }

        let deps = extract_maven_deps(&Bytes::from(buf));
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "g:a");
        assert_eq!(deps[0].version_req.as_deref(), Some("1.2.3"));
    }
}
