//! Upstream response DTOs for the JetBrains Marketplace.
//!
//! The per-plugin metadata endpoint (`/plugins/list?pluginId=`) speaks the
//! classic plugin-repository XML format — one `<idea-plugin>` element per
//! published version. The search endpoint (`/api/searchPlugins`) is JSON.

use quick_xml::{events::Event as XmlEvent, Reader as XmlReader};
use serde::Deserialize;

use batlehub_core::error::CoreError;

/// One `<idea-plugin>` element from a `/plugins/list` response — a single
/// version of the plugin.
#[derive(Debug, Clone, Default)]
pub struct PluginListEntry {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub change_notes: Option<String>,
    pub depends: Vec<String>,
    pub since_build: Option<String>,
    pub until_build: Option<String>,
    /// Publish date attribute, epoch milliseconds.
    pub date_ms: Option<i64>,
    pub size: Option<u64>,
}

fn attr_value(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    e.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
        .filter(|v| !v.is_empty())
}

fn parse_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::Registry(format!("invalid plugin-repository XML: {e}"))
}

fn assign_field(entry: &mut PluginListEntry, tag: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let text = text.to_owned();
    match tag {
        "id" => entry.id = Some(text),
        "name" => entry.name = Some(text),
        "version" => entry.version = Some(text),
        "vendor" => entry.vendor = Some(text),
        "description" => entry.description = Some(text),
        "change-notes" => entry.change_notes = Some(text),
        "depends" => entry.depends.push(text),
        _ => {}
    }
}

/// Parse a plugin-repository XML document into its `<idea-plugin>` entries.
///
/// Event state machine in the style of the NuGet `.nuspec` parser: elements
/// outside `<idea-plugin>` (`<plugin-repository>`, `<category>`) are skipped,
/// `<idea-version>` bounds come from attributes, `<depends>` repeats. Text is
/// accumulated across `Text`/`CData`/`GeneralRef` events — quick-xml emits
/// entity references (`&amp;`) as separate events — and assigned on the
/// element's `End`.
pub fn parse_plugin_list(xml: &[u8]) -> Result<Vec<PluginListEntry>, CoreError> {
    let mut reader = XmlReader::from_reader(xml);

    let mut entries = Vec::new();
    let mut current: Option<PluginListEntry> = None;
    let mut current_tag = String::new();
    let mut text_buf = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(XmlEvent::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local == "idea-plugin" {
                    current = Some(PluginListEntry {
                        date_ms: attr_value(&e, "date").and_then(|v| v.parse().ok()),
                        size: attr_value(&e, "size").and_then(|v| v.parse().ok()),
                        ..PluginListEntry::default()
                    });
                    current_tag.clear();
                } else if let Some(entry) = current.as_mut() {
                    if local == "idea-version" {
                        entry.since_build = attr_value(&e, "since-build");
                        entry.until_build = attr_value(&e, "until-build");
                    }
                    current_tag = local;
                    text_buf.clear();
                }
            }
            Ok(XmlEvent::Empty(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local == "idea-version" {
                    if let Some(entry) = current.as_mut() {
                        entry.since_build = attr_value(&e, "since-build");
                        entry.until_build = attr_value(&e, "until-build");
                    }
                }
            }
            Ok(XmlEvent::Text(e)) => {
                if current.is_some() && !current_tag.is_empty() {
                    text_buf.push_str(&e.decode().map_err(parse_err)?);
                }
            }
            Ok(XmlEvent::CData(e)) => {
                if current.is_some() && !current_tag.is_empty() {
                    text_buf.push_str(&String::from_utf8_lossy(&e));
                }
            }
            Ok(XmlEvent::GeneralRef(e)) => {
                if current.is_some() && !current_tag.is_empty() {
                    if let Some(ch) = e.resolve_char_ref().map_err(parse_err)? {
                        text_buf.push(ch);
                    } else {
                        match e.decode().map_err(parse_err)?.as_ref() {
                            "amp" => text_buf.push('&'),
                            "lt" => text_buf.push('<'),
                            "gt" => text_buf.push('>'),
                            "quot" => text_buf.push('"'),
                            "apos" => text_buf.push('\''),
                            // Unknown entity: keep it verbatim rather than dropping text.
                            other => {
                                text_buf.push('&');
                                text_buf.push_str(other);
                                text_buf.push(';');
                            }
                        }
                    }
                }
            }
            Ok(XmlEvent::End(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local == "idea-plugin" {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                } else {
                    if let Some(entry) = current.as_mut() {
                        if local == current_tag {
                            assign_field(entry, &current_tag, &text_buf);
                        }
                    }
                    current_tag.clear();
                    text_buf.clear();
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(e) => return Err(parse_err(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

// ── /api/searchPlugins JSON ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchPluginsResponse {
    #[serde(default)]
    pub plugins: Vec<SearchPluginHit>,
}

#[derive(Debug, Deserialize)]
pub struct SearchPluginHit {
    #[serde(rename = "xmlId")]
    pub xml_id: String,
    #[allow(dead_code)]
    pub name: Option<String>,
    /// Short description shown in search results.
    pub preview: Option<String>,
}
