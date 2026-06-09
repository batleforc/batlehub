use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_maven_deps(data: &Bytes) -> Vec<SbomDependency> {
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
                        _ => current_version = text,
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
