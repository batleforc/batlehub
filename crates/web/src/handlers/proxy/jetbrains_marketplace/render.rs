//! Shape per-plugin responses (plugin JSON, updates array, meta.json, XML
//! documents) from either the local registry's typed entries or the proxy
//! metadata cache's `PackageMetadata.extra` — one renderer for both sources so
//! every mode serves the same shapes.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;
use serde::Deserialize;

use batlehub_core::services::JetbrainsPluginVersion;

use crate::error::AppError;

/// One version entry from the adapter's `extra.versions` array.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtraVersion {
    pub version: Option<String>,
    pub since_build: Option<String>,
    pub until_build: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    pub date_ms: Option<i64>,
    pub size: Option<u64>,
}

/// The plugin-level fields the adapter stores in `PackageMetadata.extra`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtraMeta {
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub change_notes: Option<String>,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub versions: Vec<ExtraVersion>,
}

impl ExtraMeta {
    pub fn from_extra(extra: &serde_json::Value) -> Self {
        serde_json::from_value(extra.clone()).unwrap_or_default()
    }

    /// Newest version compatible with `build` (all versions when `build` is
    /// `None`), by publish date.
    pub fn newest_compatible(&self, build: Option<&str>) -> Option<&ExtraVersion> {
        self.versions
            .iter()
            .filter(|v| {
                build.is_none_or(|b| {
                    batlehub_core::services::build_in_range(
                        v.since_build.as_deref(),
                        v.until_build.as_deref(),
                        b,
                    )
                })
            })
            .max_by_key(|v| v.date_ms.unwrap_or(i64::MIN))
    }
}

/// Renderer-agnostic view of one plugin version, buildable from both the local
/// eco entries and the proxy `extra`.
#[derive(Debug, Clone, Default)]
pub struct RenderEntry {
    pub xml_id: String,
    pub version: String,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub change_notes: Option<String>,
    pub depends: Vec<String>,
    pub since_build: Option<String>,
    pub until_build: Option<String>,
    pub channel: String,
    pub date_ms: Option<i64>,
    pub size: Option<u64>,
    /// Self-referencing artifact URL used in `updatePlugins.xml`.
    pub download_url: Option<String>,
}

impl RenderEntry {
    pub fn from_local(v: &JetbrainsPluginVersion) -> Self {
        Self {
            xml_id: v.xml_id.clone(),
            version: v.version.clone(),
            name: v.name.clone(),
            vendor: v.vendor.clone(),
            description: v.description.clone(),
            change_notes: v.change_notes.clone(),
            depends: v.depends.clone(),
            since_build: v.since_build.clone(),
            until_build: v.until_build.clone(),
            channel: v.channel.clone(),
            date_ms: Some(v.published_at.timestamp_millis()),
            size: Some(v.size),
            download_url: None,
        }
    }

    pub fn from_extra(xml_id: &str, meta: &ExtraMeta, v: &ExtraVersion) -> Self {
        Self {
            xml_id: xml_id.to_owned(),
            version: v.version.clone().unwrap_or_default(),
            name: meta.name.clone(),
            vendor: meta.vendor.clone(),
            description: meta.description.clone(),
            change_notes: meta.change_notes.clone(),
            depends: meta.depends.clone(),
            since_build: v.since_build.clone(),
            until_build: v.until_build.clone(),
            channel: v.channel.clone().unwrap_or_default(),
            date_ms: v.date_ms,
            size: v.size,
            download_url: None,
        }
    }
}

// ── JSON shapes ───────────────────────────────────────────────────────────────

/// `/api/plugins/{id}` plugin object. String external ids: `id` carries the
/// xmlId (the IDE treats it as opaque — documented v1 tradeoff).
pub fn plugin_json(e: &RenderEntry) -> serde_json::Value {
    serde_json::json!({
        "id": e.xml_id,
        "xmlId": e.xml_id,
        "name": e.name,
        "description": e.description,
        "vendor": { "name": e.vendor },
        "version": e.version,
    })
}

/// One element of the `/api/plugins/{id}/updates` array (also the shape used
/// in compatible-updates responses).
pub fn update_json(e: &RenderEntry) -> serde_json::Value {
    serde_json::json!({
        "id": e.version,
        "pluginId": e.xml_id,
        "pluginXmlId": e.xml_id,
        "version": e.version,
        "since": e.since_build,
        "until": e.until_build,
        "channel": e.channel,
        "cdate": e.date_ms,
        "size": e.size,
    })
}

/// `/files/{plugin}/meta.json` — plugin-level blob.
pub fn plugin_meta_json(xml_id: &str, entries: &[RenderEntry]) -> serde_json::Value {
    let newest = entries.iter().max_by_key(|e| e.date_ms.unwrap_or(i64::MIN));
    serde_json::json!({
        "id": xml_id,
        "xmlId": xml_id,
        "name": newest.and_then(|e| e.name.clone()),
        "description": newest.and_then(|e| e.description.clone()),
        "vendor": { "name": newest.and_then(|e| e.vendor.clone()) },
        "versions": entries.iter().map(update_json).collect::<Vec<_>>(),
    })
}

/// `/files/{plugin}/{update}/meta.json` — update-level blob.
pub fn update_meta_json(e: &RenderEntry) -> serde_json::Value {
    serde_json::json!({
        "id": e.version,
        "xmlId": e.xml_id,
        "name": e.name,
        "version": e.version,
        "since": e.since_build,
        "until": e.until_build,
        "channel": e.channel,
        "size": e.size,
        "date": e.date_ms,
    })
}

/// Search hit in the `/api/searchPlugins` `{plugins, total}` shape.
pub fn search_hit_json(e: &RenderEntry) -> serde_json::Value {
    serde_json::json!({
        "id": e.xml_id,
        "xmlId": e.xml_id,
        "name": e.name,
        "preview": e.description,
        "downloads": 0,
        "vendor": { "name": e.vendor },
    })
}

// ── XML documents ─────────────────────────────────────────────────────────────

fn xml_err(e: impl std::fmt::Display) -> AppError {
    AppError::from(batlehub_core::error::CoreError::Other(anyhow::anyhow!(
        "xml write: {e}"
    )))
}

macro_rules! leaf {
    ($w:expr, $tag:expr, $val:expr) => {{
        $w.write_event(Event::Start(BytesStart::new($tag)))
            .map_err(xml_err)?;
        $w.write_event(Event::Text(BytesText::new($val)))
            .map_err(xml_err)?;
        $w.write_event(Event::End(BytesEnd::new($tag)))
            .map_err(xml_err)?;
    }};
}

fn opt_leaf<W: std::io::Write>(
    w: &mut Writer<W>,
    tag: &str,
    val: &Option<String>,
) -> Result<(), AppError> {
    if let Some(v) = val {
        leaf!(w, tag, v);
    }
    Ok(())
}

fn idea_version_tag(e: &RenderEntry) -> BytesStart<'static> {
    let mut t = BytesStart::new("idea-version");
    if let Some(s) = &e.since_build {
        t.push_attribute(("since-build", s.as_str()));
    }
    if let Some(u) = &e.until_build {
        t.push_attribute(("until-build", u.as_str()));
    }
    t
}

/// Classic `/plugins/list` plugin-repository document: one `<idea-plugin>` per
/// version.
pub fn plugin_repository_xml(entries: &[RenderEntry]) -> Result<String, AppError> {
    let mut buf = Vec::new();
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(xml_err)?;
    w.write_event(Event::Start(BytesStart::new("plugin-repository")))
        .map_err(xml_err)?;
    let mut category = BytesStart::new("category");
    category.push_attribute(("name", "Plugins"));
    w.write_event(Event::Start(category)).map_err(xml_err)?;

    for e in entries {
        let mut plugin = BytesStart::new("idea-plugin");
        if let Some(d) = e.date_ms {
            plugin.push_attribute(("date", d.to_string().as_str()));
        }
        if let Some(s) = e.size {
            plugin.push_attribute(("size", s.to_string().as_str()));
        }
        w.write_event(Event::Start(plugin)).map_err(xml_err)?;

        leaf!(w, "id", &e.xml_id);
        opt_leaf(&mut w, "name", &e.name)?;
        leaf!(w, "version", &e.version);
        w.write_event(Event::Empty(idea_version_tag(e)))
            .map_err(xml_err)?;
        opt_leaf(&mut w, "vendor", &e.vendor)?;
        opt_leaf(&mut w, "description", &e.description)?;
        opt_leaf(&mut w, "change-notes", &e.change_notes)?;
        for dep in &e.depends {
            leaf!(w, "depends", dep);
        }

        w.write_event(Event::End(BytesEnd::new("idea-plugin")))
            .map_err(xml_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("category")))
        .map_err(xml_err)?;
    w.write_event(Event::End(BytesEnd::new("plugin-repository")))
        .map_err(xml_err)?;

    String::from_utf8(buf)
        .map_err(|e| AppError::from(batlehub_core::error::CoreError::Other(anyhow::anyhow!(e))))
}

/// `updatePlugins.xml` custom-repository document (`idea.plugin.hosts` flow):
/// one `<plugin>` per plugin with a self-referencing download `url`.
pub fn update_plugins_xml(entries: &[RenderEntry]) -> Result<String, AppError> {
    let mut buf = Vec::new();
    let mut w = Writer::new_with_indent(&mut buf, b' ', 2);

    w.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(xml_err)?;
    w.write_event(Event::Start(BytesStart::new("plugins")))
        .map_err(xml_err)?;

    for e in entries {
        let mut plugin = BytesStart::new("plugin");
        plugin.push_attribute(("id", e.xml_id.as_str()));
        if let Some(url) = &e.download_url {
            plugin.push_attribute(("url", url.as_str()));
        }
        plugin.push_attribute(("version", e.version.as_str()));
        w.write_event(Event::Start(plugin)).map_err(xml_err)?;

        w.write_event(Event::Empty(idea_version_tag(e)))
            .map_err(xml_err)?;
        opt_leaf(&mut w, "name", &e.name)?;
        opt_leaf(&mut w, "vendor", &e.vendor)?;
        opt_leaf(&mut w, "description", &e.description)?;
        opt_leaf(&mut w, "change-notes", &e.change_notes)?;
        for dep in &e.depends {
            leaf!(w, "depends", dep);
        }

        w.write_event(Event::End(BytesEnd::new("plugin")))
            .map_err(xml_err)?;
    }

    w.write_event(Event::End(BytesEnd::new("plugins")))
        .map_err(xml_err)?;

    String::from_utf8(buf)
        .map_err(|e| AppError::from(batlehub_core::error::CoreError::Other(anyhow::anyhow!(e))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> RenderEntry {
        RenderEntry {
            xml_id: "org.example.plugin".into(),
            version: "1.2.0".into(),
            name: Some("Example".into()),
            vendor: Some("Acme & Co".into()),
            description: Some("A <great> plugin".into()),
            change_notes: None,
            depends: vec!["com.intellij.modules.platform".into()],
            since_build: Some("233.0".into()),
            until_build: Some("241.*".into()),
            channel: String::new(),
            date_ms: Some(1_700_000_000_000),
            size: Some(2048),
            download_url: Some("https://proxy/dl".into()),
        }
    }

    #[test]
    fn update_plugins_xml_shape() {
        let xml = update_plugins_xml(&[entry()]).unwrap();
        assert!(xml.contains(
            r#"<plugin id="org.example.plugin" url="https://proxy/dl" version="1.2.0">"#
        ));
        assert!(xml.contains(r#"<idea-version since-build="233.0" until-build="241.*"/>"#));
        assert!(xml.contains("<name>Example</name>"));
        // Reserved characters must be escaped.
        assert!(xml.contains("Acme &amp; Co"));
        assert!(xml.contains("A &lt;great&gt; plugin"));
    }

    #[test]
    fn attribute_values_are_escaped() {
        // quick-xml's `push_attribute((&str, &str))` escapes the value (`"`,
        // `&`, `<` …) — hostile descriptor fields cannot break out of the
        // generated attributes.
        let mut e = entry();
        e.until_build = Some(r#"241.*" injected="x"#.into());
        e.download_url = Some("https://proxy/dl?a=1&b=<2>".into());
        let xml = update_plugins_xml(&[e]).unwrap();
        assert!(xml.contains(r#"until-build="241.*&quot; injected=&quot;x""#));
        assert!(xml.contains(r#"url="https://proxy/dl?a=1&amp;b=&lt;2&gt;""#));
        assert!(!xml.contains(r#"injected="x""#));
    }

    #[test]
    fn plugin_repository_xml_shape() {
        let xml = plugin_repository_xml(&[entry()]).unwrap();
        assert!(xml.contains("<plugin-repository>"));
        assert!(xml.contains(r#"<idea-plugin date="1700000000000" size="2048">"#));
        assert!(xml.contains("<id>org.example.plugin</id>"));
        assert!(xml.contains("<version>1.2.0</version>"));
        assert!(xml.contains("<depends>com.intellij.modules.platform</depends>"));
    }

    #[test]
    fn extra_meta_round_trip_and_compat() {
        let extra = serde_json::json!({
            "name": "Example",
            "vendor": "Acme",
            "versions": [
                {"version": "1.2.0", "since_build": "241.0", "until_build": "242.*", "date_ms": 200},
                {"version": "1.1.0", "since_build": "233.0", "until_build": "240.*", "date_ms": 100},
            ],
        });
        let meta = ExtraMeta::from_extra(&extra);
        assert_eq!(meta.versions.len(), 2);
        let best = meta.newest_compatible(Some("IU-241.100")).unwrap();
        assert_eq!(best.version.as_deref(), Some("1.2.0"));
        let older = meta.newest_compatible(Some("234.0")).unwrap();
        assert_eq!(older.version.as_deref(), Some("1.1.0"));
        assert!(meta.newest_compatible(Some("999.0")).is_none());
    }

    #[test]
    fn malformed_extra_degrades_to_default() {
        let meta = ExtraMeta::from_extra(&serde_json::json!("not an object"));
        assert!(meta.versions.is_empty());
    }
}
