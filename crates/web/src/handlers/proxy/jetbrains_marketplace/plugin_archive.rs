//! Plugin descriptor extraction from uploaded archives.
//!
//! A JetBrains plugin ships either as a `.jar` (with `META-INF/plugin.xml`
//! inside) or as a `.zip` distribution containing `<name>/lib/*.jar`, one of
//! which carries the descriptor. All reads are bounded (zip-bomb guard): the
//! compressed upload itself is capped by the multipart accumulation limit in
//! `publish.rs`, but a hostile archive could still declare entries that
//! decompress far larger — hence the per-jar and cumulative ceilings below.

use std::io::Read;

use quick_xml::{events::Event as XmlEvent, Reader as XmlReader};

use crate::error::AppError;

/// Decompressed ceiling for a nested `lib/*.jar` read while hunting for the
/// descriptor. Descriptor-bearing jars are typically a few MiB; the ceiling
/// stays generous for fat plugin jars without allowing quarter-GiB blowups.
const MAX_NESTED_JAR_BYTES: u64 = 128 * 1024 * 1024;
/// Cumulative decompressed budget across all nested jars of one archive, so a
/// zip with many large entries cannot force sustained per-request heap churn.
const MAX_TOTAL_NESTED_BYTES: u64 = 512 * 1024 * 1024;
/// Decompressed ceiling for `META-INF/plugin.xml` itself.
const MAX_DESCRIPTOR_BYTES: u64 = 10 * 1024 * 1024;

/// Fields parsed from `META-INF/plugin.xml`.
#[derive(Debug, Default, Clone)]
pub struct PluginDescriptor {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub change_notes: Option<String>,
    pub depends: Vec<String>,
    pub since_build: Option<String>,
    pub until_build: Option<String>,
}

/// Extract and parse the plugin descriptor from an uploaded archive
/// (`.jar` direct, or `.zip` with nested `*/lib/*.jar`).
pub fn extract_plugin_descriptor(bytes: &[u8]) -> Result<PluginDescriptor, AppError> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| AppError::unprocessable(format!("invalid plugin archive (not a ZIP): {e}")))?;

    // Case 1: the archive is itself the plugin jar.
    if let Some(xml) = read_descriptor_entry(&mut archive)? {
        return parse_plugin_xml(&xml);
    }

    // Case 2: zip distribution — scan nested lib/*.jar entries for the
    // descriptor. Bounded read per jar; a jar without the descriptor is skipped.
    let mut total_nested: u64 = 0;
    for i in 0..archive.len() {
        let is_lib_jar = {
            let entry = archive
                .by_index(i)
                .map_err(|e| AppError::unprocessable(format!("zip entry error: {e}")))?;
            entry.name().contains("/lib/") && entry.name().ends_with(".jar")
        };
        if !is_lib_jar {
            continue;
        }
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::unprocessable(format!("zip entry error: {e}")))?;
        let mut jar_bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_NESTED_JAR_BYTES + 1)
            .read_to_end(&mut jar_bytes)
            .map_err(|e| AppError::unprocessable(format!("reading nested jar: {e}")))?;
        if jar_bytes.len() as u64 > MAX_NESTED_JAR_BYTES {
            return Err(AppError::unprocessable(format!(
                "nested jar exceeds the {MAX_NESTED_JAR_BYTES}-byte decompressed limit"
            )));
        }
        total_nested += jar_bytes.len() as u64;
        if total_nested > MAX_TOTAL_NESTED_BYTES {
            return Err(AppError::unprocessable(format!(
                "nested jars exceed the {MAX_TOTAL_NESTED_BYTES}-byte cumulative decompressed limit"
            )));
        }
        let Ok(mut jar) = zip::ZipArchive::new(std::io::Cursor::new(jar_bytes.as_slice())) else {
            continue;
        };
        if let Some(xml) = read_descriptor_entry(&mut jar)? {
            return parse_plugin_xml(&xml);
        }
    }

    Err(AppError::unprocessable(
        "no META-INF/plugin.xml found in the plugin archive",
    ))
}

/// Read `META-INF/plugin.xml` from an open archive, bounded.
fn read_descriptor_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Option<Vec<u8>>, AppError> {
    let Some(index) = (0..archive.len()).find(|&i| {
        archive
            .name_for_index(i)
            .is_some_and(|n| n == "META-INF/plugin.xml")
    }) else {
        return Ok(None);
    };
    let mut entry = archive
        .by_index(index)
        .map_err(|e| AppError::unprocessable(format!("zip entry error: {e}")))?;
    let mut buf = Vec::new();
    entry
        .by_ref()
        .take(MAX_DESCRIPTOR_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| AppError::unprocessable(format!("reading plugin.xml: {e}")))?;
    if buf.len() as u64 > MAX_DESCRIPTOR_BYTES {
        return Err(AppError::unprocessable(format!(
            "plugin.xml exceeds the {MAX_DESCRIPTOR_BYTES}-byte limit"
        )));
    }
    Ok(Some(buf))
}

/// Parse `plugin.xml` — same accumulate-text state machine as the adapter's
/// plugin-repository parser (entities arrive as separate events in quick-xml).
fn parse_plugin_xml(xml: &[u8]) -> Result<PluginDescriptor, AppError> {
    fn parse_err(e: impl std::fmt::Display) -> AppError {
        AppError::unprocessable(format!("plugin.xml parse: {e}"))
    }

    let mut reader = XmlReader::from_reader(xml);
    let mut d = PluginDescriptor::default();
    let mut current_tag = String::new();
    let mut text_buf = String::new();
    let mut depth = 0u32;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                depth += 1;
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                // Only direct children of <idea-plugin> are descriptor fields;
                // nested extension blocks are skipped.
                if depth == 2 {
                    if local == "idea-version" {
                        d.since_build = attr(&e, "since-build");
                        d.until_build = attr(&e, "until-build");
                    }
                    current_tag = local;
                    text_buf.clear();
                } else if depth != 2 {
                    current_tag.clear();
                }
            }
            Ok(XmlEvent::Empty(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if depth == 1 && local == "idea-version" {
                    d.since_build = attr(&e, "since-build");
                    d.until_build = attr(&e, "until-build");
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if depth == 2 && !current_tag.is_empty() {
                    text_buf.push_str(&e.decode().map_err(parse_err)?);
                }
            }
            Ok(XmlEvent::CData(e)) => {
                if depth == 2 && !current_tag.is_empty() {
                    text_buf.push_str(&String::from_utf8_lossy(&e));
                }
            }
            Ok(XmlEvent::GeneralRef(e)) => {
                if depth == 2 && !current_tag.is_empty() {
                    if let Some(ch) = e.resolve_char_ref().map_err(parse_err)? {
                        text_buf.push(ch);
                    } else {
                        match e.decode().map_err(parse_err)?.as_ref() {
                            "amp" => text_buf.push('&'),
                            "lt" => text_buf.push('<'),
                            "gt" => text_buf.push('>'),
                            "quot" => text_buf.push('"'),
                            "apos" => text_buf.push('\''),
                            other => {
                                text_buf.push('&');
                                text_buf.push_str(other);
                                text_buf.push(';');
                            }
                        }
                    }
                }
            }
            Ok(XmlEvent::End(_)) => {
                if depth == 2 {
                    let text = text_buf.trim();
                    if !text.is_empty() {
                        let text = text.to_owned();
                        match current_tag.as_str() {
                            "id" => d.id = Some(text),
                            "name" => d.name = Some(text),
                            "version" => d.version = Some(text),
                            "vendor" => d.vendor = Some(text),
                            "description" => d.description = Some(text),
                            "change-notes" => d.change_notes = Some(text),
                            "depends" => d.depends.push(text),
                            _ => {}
                        }
                    }
                    current_tag.clear();
                    text_buf.clear();
                }
                depth = depth.saturating_sub(1);
            }
            Ok(XmlEvent::Eof) => break,
            Err(e) => return Err(parse_err(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(d)
}

fn attr(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    e.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    pub(crate) const PLUGIN_XML: &str = r#"<idea-plugin>
  <id>org.example.demo</id>
  <name>Demo &amp; More</name>
  <version>1.0.0</version>
  <vendor email="dev@example.com">Example</vendor>
  <description><![CDATA[A demo plugin]]></description>
  <change-notes>First release</change-notes>
  <depends>com.intellij.modules.platform</depends>
  <idea-version since-build="233.0" until-build="241.*"/>
  <extensions defaultExtensionNs="com.intellij">
    <id>should-not-leak-into-descriptor</id>
  </extensions>
</idea-plugin>"#;

    fn make_jar(descriptor: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file("META-INF/plugin.xml", SimpleFileOptions::default())
            .unwrap();
        w.write_all(descriptor.as_bytes()).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    fn make_zip_dist(descriptor: &str) -> Vec<u8> {
        let jar = make_jar(descriptor);
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file("demo/lib/other.txt", SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"not a jar").unwrap();
        w.start_file("demo/lib/demo.jar", SimpleFileOptions::default())
            .unwrap();
        w.write_all(&jar).unwrap();
        w.finish().unwrap();
        buf.into_inner()
    }

    #[test]
    fn extracts_descriptor_from_jar() {
        let d = extract_plugin_descriptor(&make_jar(PLUGIN_XML)).unwrap();
        assert_eq!(d.id.as_deref(), Some("org.example.demo"));
        assert_eq!(d.name.as_deref(), Some("Demo & More"));
        assert_eq!(d.version.as_deref(), Some("1.0.0"));
        assert_eq!(d.vendor.as_deref(), Some("Example"));
        assert_eq!(d.description.as_deref(), Some("A demo plugin"));
        assert_eq!(d.change_notes.as_deref(), Some("First release"));
        assert_eq!(d.depends, vec!["com.intellij.modules.platform"]);
        assert_eq!(d.since_build.as_deref(), Some("233.0"));
        assert_eq!(d.until_build.as_deref(), Some("241.*"));
    }

    #[test]
    fn extracts_descriptor_from_nested_zip() {
        let d = extract_plugin_descriptor(&make_zip_dist(PLUGIN_XML)).unwrap();
        assert_eq!(d.id.as_deref(), Some("org.example.demo"));
        assert_eq!(d.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn nested_ids_do_not_overwrite_descriptor_fields() {
        let d = extract_plugin_descriptor(&make_jar(PLUGIN_XML)).unwrap();
        assert_eq!(
            d.id.as_deref(),
            Some("org.example.demo"),
            "extension-block <id> must not leak"
        );
    }

    #[test]
    fn not_a_zip_is_unprocessable() {
        assert!(extract_plugin_descriptor(b"not a zip at all").is_err());
    }

    #[test]
    fn zip_without_descriptor_is_unprocessable() {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut buf);
        w.start_file("readme.txt", SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"hello").unwrap();
        w.finish().unwrap();
        assert!(extract_plugin_descriptor(&buf.into_inner()).is_err());
    }
}
