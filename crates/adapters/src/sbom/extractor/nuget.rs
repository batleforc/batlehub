use batlehub_core::ports::SbomDependency;
use bytes::Bytes;

pub(super) fn extract_nuget_deps(data: &Bytes) -> Vec<SbomDependency> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let deps = extract_nuget_deps(&data);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "Newtonsoft.Json");
    }

    #[test]
    fn extract_nuget_deps_invalid_zip() {
        let deps = extract_nuget_deps(&Bytes::from_static(b"not a zip"));
        assert!(deps.is_empty());
    }
}
