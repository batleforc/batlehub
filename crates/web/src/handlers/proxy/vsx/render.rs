//! Turning entries into the two protocols' documents. Pure — no actix, no
//! services, no I/O — so every shape here is unit-testable.
//!
//! One intermediate, [`GalleryEntry`], is produced by both the local and the
//! proxy path, and every document is a function of it. That is the same
//! arrangement `jetbrains_marketplace/render.rs` uses, and it is what lets
//! blocked-version filtering happen once, upstream of all of them.

use serde_json::{json, Value};

use super::protocol::{asset_type, flag, property_key, GalleryQuery};
use batlehub_core::services::OpenVsxExtensionVersion;

/// One extension, with the versions that survived filtering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GalleryEntry {
    pub publisher: String,
    pub extension_name: String,
    /// The stable identity the editor keys installs on. See
    /// [`synthetic_extension_uuid`].
    pub extension_id: String,
    pub display_name: Option<String>,
    pub short_description: Option<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub install_count: u64,
    pub versions: Vec<GalleryVersion>,
    /// The upstream extension object this was parsed from, when there was one.
    ///
    /// Rendering starts from this and overwrites only the fields the proxy owns
    /// (`versions`, and the URLs inside them). Everything the marketplace sends
    /// that this proxy has never modelled — properties, statistics, flags that
    /// vary by marketplace version — therefore survives the round trip. A
    /// parse-and-rebuild would silently drop them, and the loss would only show
    /// up as a subtly wrong extension pane.
    pub upstream: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GalleryVersion {
    pub version: String,
    /// RFC 3339.
    pub last_updated: String,
    /// `engines.vscode`. Absent is **not** the same as permissive: the editor
    /// falls back to reading the manifest asset, which this server serves.
    pub engine: Option<String>,
    pub extension_pack: Vec<String>,
    pub extension_dependencies: Vec<String>,
    pub pre_release: bool,
}

/// `{publisher}.{name}`.
pub fn qualified_name(publisher: &str, name: &str) -> String {
    format!("{publisher}.{name}")
}

/// A deterministic UUID for an extension that has no upstream one.
///
/// VS Code stores this as the extension's identity and uses it to detect
/// renames and to deduplicate installs, so it has to be **stable across
/// restarts and across every BatleHub instance** — a random one would make the
/// editor believe a locally published extension changed identity on every
/// server bounce.
///
/// UUIDv5 over the qualified name gives exactly that, with no state to keep.
pub fn synthetic_extension_uuid(publisher: &str, name: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        qualified_name(publisher, name).as_bytes(),
    )
    .to_string()
}

impl GalleryEntry {
    /// Build from the local registry's typed versions, newest first.
    ///
    /// `versions` must be non-empty; callers get them from
    /// `get_openvsx_versions`, which errors rather than returning an empty
    /// list.
    pub fn from_local(versions: &[OpenVsxExtensionVersion]) -> Option<Self> {
        let newest = versions.iter().max_by_key(|v| v.published_at)?;
        let mut ordered: Vec<&OpenVsxExtensionVersion> = versions.iter().collect();
        ordered.sort_by_key(|v| std::cmp::Reverse(v.published_at));

        Some(Self {
            publisher: newest.publisher.clone(),
            extension_name: newest.name.clone(),
            extension_id: synthetic_extension_uuid(&newest.publisher, &newest.name),
            display_name: newest.display_name.clone(),
            short_description: newest.description.clone(),
            categories: newest.categories.clone(),
            tags: newest.tags.clone(),
            // A private registry keeps no install counter. Reported as zero
            // rather than omitted, because the editor sorts by it and a missing
            // key reads as malformed.
            install_count: 0,
            versions: ordered
                .into_iter()
                .map(|v| GalleryVersion {
                    version: v.version.clone(),
                    last_updated: v.published_at.to_rfc3339(),
                    engine: v.engine.clone(),
                    extension_pack: v.extension_pack.clone(),
                    extension_dependencies: v.dependencies.clone(),
                    pre_release: v.pre_release,
                })
                .collect(),
            upstream: None,
        })
    }

    /// Parse one `extensionquery` extension object from an upstream response.
    pub fn from_upstream(obj: &Value) -> Option<Self> {
        let publisher = obj
            .get("publisher")
            .and_then(|p| p.get("publisherName"))
            .and_then(Value::as_str)?
            .to_owned();
        let extension_name = obj.get("extensionName").and_then(Value::as_str)?.to_owned();

        let versions = obj
            .get("versions")
            .and_then(Value::as_array)
            .map(|vs| {
                vs.iter()
                    .filter_map(GalleryVersion::from_upstream)
                    .collect()
            })
            .unwrap_or_default();

        Some(Self {
            extension_id: obj
                .get("extensionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| synthetic_extension_uuid(&publisher, &extension_name)),
            display_name: obj
                .get("displayName")
                .and_then(Value::as_str)
                .map(str::to_owned),
            short_description: obj
                .get("shortDescription")
                .and_then(Value::as_str)
                .map(str::to_owned),
            categories: string_list(obj.get("categories")),
            tags: string_list(obj.get("tags")),
            install_count: obj
                .get("statistics")
                .and_then(Value::as_array)
                .and_then(|stats| {
                    stats
                        .iter()
                        .find(|s| s.get("statisticName").and_then(Value::as_str) == Some("install"))
                        .and_then(|s| s.get("value"))
                        .and_then(Value::as_f64)
                })
                .unwrap_or(0.0) as u64,
            publisher,
            extension_name,
            versions,
            upstream: Some(obj.clone()),
        })
    }

    pub fn qualified_name(&self) -> String {
        qualified_name(&self.publisher, &self.extension_name)
    }

    /// Whether `query`'s free-text and facet criteria select this entry.
    ///
    /// Substring, case-insensitive, over the fields a user would type.
    /// Deliberately not ranked: a private registry holds tens of extensions,
    /// and a relevance model nobody can explain is worse than an obvious one.
    pub fn matches(&self, query: &GalleryQuery) -> bool {
        if !query.extension_names.is_empty()
            && !query
                .extension_names
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&self.qualified_name()))
        {
            return false;
        }
        if !query.extension_ids.is_empty()
            && !query
                .extension_ids
                .iter()
                .any(|id| id.eq_ignore_ascii_case(&self.extension_id))
        {
            return false;
        }
        if !query
            .tags
            .iter()
            .all(|t| self.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
        {
            return false;
        }
        if !query
            .categories
            .iter()
            .all(|c| self.categories.iter().any(|x| x.eq_ignore_ascii_case(c)))
        {
            return false;
        }
        if let Some(text) = query.search_text.as_deref() {
            let q = text.to_lowercase();
            let hit = |s: &str| s.to_lowercase().contains(&q);
            let found = hit(&self.qualified_name())
                || self.display_name.as_deref().is_some_and(hit)
                || self.short_description.as_deref().is_some_and(hit)
                || self.tags.iter().any(|t| hit(t))
                || self.categories.iter().any(|c| hit(c));
            if !found {
                return false;
            }
        }
        true
    }
}

impl GalleryVersion {
    fn from_upstream(v: &Value) -> Option<Self> {
        let props = v.get("properties").and_then(Value::as_array);
        let prop = |key: &str| -> Option<String> {
            props?
                .iter()
                .find(|p| p.get("key").and_then(Value::as_str) == Some(key))
                .and_then(|p| p.get("value"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let csv = |key: &str| -> Vec<String> {
            prop(key)
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };

        Some(Self {
            version: v.get("version").and_then(Value::as_str)?.to_owned(),
            last_updated: v
                .get("lastUpdated")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            engine: prop(property_key::ENGINE),
            extension_pack: csv(property_key::EXTENSION_PACK),
            extension_dependencies: csv(property_key::EXTENSION_DEPENDENCIES),
            pre_release: prop(property_key::PRE_RELEASE).as_deref() == Some("true"),
        })
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

// ── Documents ────────────────────────────────────────────────────────────────

/// Where an editor should fetch this registry's assets from.
///
/// Built from `registry_public_base`, never from a literal path — that is the
/// one function that gets host routing and the forwarded-header trust boundary
/// right.
#[derive(Debug, Clone)]
pub struct GalleryUrls {
    base: String,
}

impl GalleryUrls {
    pub fn new(public_base: &str) -> Self {
        Self {
            base: public_base.trim_end_matches('/').to_owned(),
        }
    }

    /// `assetUri` — the editor appends `/{assetType}` to this.
    pub fn asset_base(&self, publisher: &str, name: &str, version: &str) -> String {
        format!("{}/vscode/asset/{publisher}/{name}/{version}", self.base)
    }

    pub fn asset(&self, publisher: &str, name: &str, version: &str, asset_type: &str) -> String {
        format!("{}/{asset_type}", self.asset_base(publisher, name, version))
    }
}

/// One extension object for an `extensionquery` response.
///
/// Starts from `entry.upstream` when there is one and overwrites the fields the
/// proxy owns, so unmodelled upstream keys survive. The overwritten set is
/// closed and small: `versions` (filtered, and re-pointed at this host),
/// `publisher`, `extensionName`, `extensionId`, and the flag-gated blocks.
pub fn extension_json(entry: &GalleryEntry, urls: &GalleryUrls, query: &GalleryQuery) -> Value {
    let mut out = match &entry.upstream {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let obj = out.as_object_mut().expect("object");

    obj.insert(
        "publisher".to_owned(),
        json!({
            "publisherName": entry.publisher,
            "displayName": entry.publisher,
        }),
    );
    obj.insert("extensionId".to_owned(), json!(entry.extension_id));
    obj.insert("extensionName".to_owned(), json!(entry.extension_name));
    obj.insert(
        "displayName".to_owned(),
        json!(entry
            .display_name
            .clone()
            .unwrap_or_else(|| entry.extension_name.clone())),
    );
    obj.insert(
        "shortDescription".to_owned(),
        json!(entry.short_description.clone().unwrap_or_default()),
    );

    if query.wants(flag::INCLUDE_CATEGORY_AND_TAGS) {
        obj.insert("categories".to_owned(), json!(entry.categories));
        obj.insert("tags".to_owned(), json!(entry.tags));
    } else {
        obj.insert("categories".to_owned(), json!([]));
        obj.insert("tags".to_owned(), json!([]));
    }

    if query.wants(flag::INCLUDE_STATISTICS) {
        obj.insert(
            "statistics".to_owned(),
            json!([
                { "statisticName": "install", "value": entry.install_count },
                { "statisticName": "averagerating", "value": 0 },
                { "statisticName": "ratingcount", "value": 0 },
            ]),
        );
    }

    // `IncludeLatestVersionOnly` is applied **after** blocked versions were
    // removed upstream of here. Truncating first would offer the editor a
    // blocked build as *the* version, and the install would then be refused —
    // the exact failure hiding exists to prevent.
    let versions: Vec<&GalleryVersion> = if query.wants(flag::INCLUDE_LATEST_VERSION_ONLY) {
        entry.versions.iter().take(1).collect()
    } else {
        entry.versions.iter().collect()
    };

    if query.wants(flag::INCLUDE_VERSIONS) {
        obj.insert(
            "versions".to_owned(),
            json!(versions
                .iter()
                .map(|v| version_json(entry, v, urls, query))
                .collect::<Vec<_>>()),
        );
    } else {
        obj.insert("versions".to_owned(), json!([]));
    }

    out
}

fn version_json(
    entry: &GalleryEntry,
    v: &GalleryVersion,
    urls: &GalleryUrls,
    query: &GalleryQuery,
) -> Value {
    let asset_base = urls.asset_base(&entry.publisher, &entry.extension_name, &v.version);

    let mut out = json!({
        "version": v.version,
        "lastUpdated": v.last_updated,
        // Emitted regardless of `IncludeAssetUri`. A client that did not ask
        // ignores two strings; a client that needed them and forgot the flag
        // would otherwise have no way to reach any asset at all, and a
        // hand-written product.json gets flags wrong far more often than it
        // gets URLs wrong.
        "assetUri": asset_base,
        "fallbackAssetUri": asset_base,
    });
    let obj = out.as_object_mut().expect("object");

    if query.wants(flag::INCLUDE_FILES) {
        obj.insert(
            "files".to_owned(),
            json!(asset_type::ALL
                .iter()
                .map(|t| json!({
                    "assetType": t,
                    "source": urls.asset(&entry.publisher, &entry.extension_name, &v.version, t),
                }))
                .collect::<Vec<_>>()),
        );
    } else {
        obj.insert("files".to_owned(), json!([]));
    }

    if query.wants(flag::INCLUDE_VERSION_PROPERTIES) {
        let mut props = Vec::new();
        if let Some(engine) = &v.engine {
            props.push(json!({ "key": property_key::ENGINE, "value": engine }));
        }
        if !v.extension_pack.is_empty() {
            props.push(json!({
                "key": property_key::EXTENSION_PACK,
                "value": v.extension_pack.join(","),
            }));
        }
        if !v.extension_dependencies.is_empty() {
            props.push(json!({
                "key": property_key::EXTENSION_DEPENDENCIES,
                "value": v.extension_dependencies.join(","),
            }));
        }
        if v.pre_release {
            props.push(json!({ "key": property_key::PRE_RELEASE, "value": "true" }));
        }
        obj.insert("properties".to_owned(), json!(props));
    }

    out
}

/// The `extensionquery` response envelope.
///
/// `total` is the count **before** paging and **after** filtering — the editor
/// uses it to decide whether to ask for another page, so a count that ignores
/// either stops "load more" a page early or spins forever on an empty one.
pub fn query_response_json(
    entries: &[GalleryEntry],
    total: usize,
    urls: &GalleryUrls,
    query: &GalleryQuery,
) -> Value {
    json!({
        "results": [{
            "extensions": entries
                .iter()
                .map(|e| extension_json(e, urls, query))
                .collect::<Vec<_>>(),
            "pagingToken": Value::Null,
            "resultMetadata": [{
                "metadataType": "ResultCount",
                "metadataItems": [{ "name": "TotalCount", "count": total }],
            }],
        }]
    })
}

/// One extension for the OpenVSX native API (`/api/{namespace}/{extension}`).
pub fn openvsx_extension_json(
    entry: &GalleryEntry,
    version: &GalleryVersion,
    all_versions: &[GalleryVersion],
    urls: &GalleryUrls,
) -> Value {
    let file = |t: &str| urls.asset(&entry.publisher, &entry.extension_name, &version.version, t);
    json!({
        "namespace": entry.publisher,
        "name": entry.extension_name,
        "version": version.version,
        "timestamp": version.last_updated,
        "displayName": entry.display_name,
        "description": entry.short_description,
        "categories": entry.categories,
        "tags": entry.tags,
        "engines": { "vscode": version.engine },
        "downloadCount": entry.install_count,
        "files": {
            "download": file(asset_type::VSIX_PACKAGE),
            "manifest": file(asset_type::MANIFEST),
            "readme":   file(asset_type::DETAILS),
            "changelog": file(asset_type::CHANGELOG),
            "license":  file(asset_type::LICENSE),
            "icon":     file(asset_type::ICON),
        },
        "allVersions": all_versions
            .iter()
            .map(|v| {
                (
                    v.version.clone(),
                    Value::String(format!(
                        "{}/api/{}/{}/{}",
                        urls.base, entry.publisher, entry.extension_name, v.version
                    )),
                )
            })
            .collect::<serde_json::Map<_, _>>(),
    })
}

/// The OpenVSX `/api/-/search` envelope.
pub fn openvsx_search_json(entries: &[GalleryEntry], total: usize, urls: &GalleryUrls) -> Value {
    json!({
        "offset": 0,
        "totalSize": total,
        "extensions": entries
            .iter()
            .filter_map(|e| e.versions.first().map(|v| (e, v)))
            .map(|(e, v)| json!({
                "url": format!("{}/api/{}/{}", urls.base, e.publisher, e.extension_name),
                "files": {
                    "download": urls.asset(&e.publisher, &e.extension_name, &v.version, asset_type::VSIX_PACKAGE),
                    "icon": urls.asset(&e.publisher, &e.extension_name, &v.version, asset_type::ICON),
                },
                "name": e.extension_name,
                "namespace": e.publisher,
                "version": v.version,
                "timestamp": v.last_updated,
                "displayName": e.display_name,
                "description": e.short_description,
                "downloadCount": e.install_count,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::proxy::vsx::protocol::ExtensionQueryRequest;

    const ALL_FLAGS: u32 = flag::INCLUDE_VERSIONS
        | flag::INCLUDE_FILES
        | flag::INCLUDE_CATEGORY_AND_TAGS
        | flag::INCLUDE_VERSION_PROPERTIES
        | flag::INCLUDE_ASSET_URI
        | flag::INCLUDE_STATISTICS;

    fn urls() -> GalleryUrls {
        GalleryUrls::new("https://hub.example.com/proxy/vsx1/")
    }

    fn q(flags: u32) -> GalleryQuery {
        GalleryQuery::from_request(&ExtensionQueryRequest {
            flags,
            ..Default::default()
        })
    }

    fn entry() -> GalleryEntry {
        GalleryEntry {
            publisher: "acme".to_owned(),
            extension_name: "tool".to_owned(),
            extension_id: synthetic_extension_uuid("acme", "tool"),
            display_name: Some("Acme Tool".to_owned()),
            short_description: Some("Does the thing".to_owned()),
            categories: vec!["Linters".to_owned()],
            tags: vec!["acme".to_owned()],
            install_count: 0,
            versions: vec![
                GalleryVersion {
                    version: "2.0.0".to_owned(),
                    last_updated: "2024-03-01T00:00:00+00:00".to_owned(),
                    engine: Some("^1.85.0".to_owned()),
                    extension_pack: vec!["acme.other".to_owned()],
                    extension_dependencies: vec![],
                    pre_release: false,
                },
                GalleryVersion {
                    version: "1.0.0".to_owned(),
                    last_updated: "2024-01-01T00:00:00+00:00".to_owned(),
                    engine: Some("^1.80.0".to_owned()),
                    extension_pack: vec![],
                    extension_dependencies: vec![],
                    pre_release: false,
                },
            ],
            upstream: None,
        }
    }

    #[test]
    fn the_uuid_is_stable_and_distinct_per_extension() {
        assert_eq!(
            synthetic_extension_uuid("acme", "tool"),
            synthetic_extension_uuid("acme", "tool"),
            "the editor keys installs on this; it cannot change between restarts"
        );
        assert_ne!(
            synthetic_extension_uuid("acme", "tool"),
            synthetic_extension_uuid("acme", "other")
        );
    }

    #[test]
    fn every_generated_url_points_at_this_host() {
        let doc = extension_json(&entry(), &urls(), &q(ALL_FLAGS));
        let v = &doc["versions"][0];

        assert_eq!(
            v["assetUri"], "https://hub.example.com/proxy/vsx1/vscode/asset/acme/tool/2.0.0",
            "the trailing slash on the base must not double up"
        );
        assert_eq!(v["assetUri"], v["fallbackAssetUri"]);

        let vsix = v["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["assetType"] == asset_type::VSIX_PACKAGE)
            .unwrap();
        assert_eq!(
            vsix["source"],
            "https://hub.example.com/proxy/vsx1/vscode/asset/acme/tool/2.0.0/\
             Microsoft.VisualStudio.Services.VSIXPackage"
        );
    }

    /// Asset URIs go out whether or not the flag asked for them — a
    /// hand-written `product.json` gets flags wrong far more often than URLs.
    #[test]
    fn asset_uris_are_emitted_even_without_the_flag() {
        let doc = extension_json(&entry(), &urls(), &q(flag::INCLUDE_VERSIONS));
        assert!(doc["versions"][0]["assetUri"].is_string());
    }

    #[test]
    fn all_six_asset_types_are_advertised() {
        let doc = extension_json(&entry(), &urls(), &q(ALL_FLAGS));
        let types: Vec<&str> = doc["versions"][0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["assetType"].as_str().unwrap())
            .collect();
        assert_eq!(types, asset_type::ALL);
    }

    #[test]
    fn version_properties_carry_the_engine_range() {
        let doc = extension_json(&entry(), &urls(), &q(ALL_FLAGS));
        let props = doc["versions"][0]["properties"].as_array().unwrap();
        let engine = props
            .iter()
            .find(|p| p["key"] == property_key::ENGINE)
            .expect("the editor refuses a version whose engine it cannot read");
        assert_eq!(engine["value"], "^1.85.0");
    }

    #[test]
    fn latest_version_only_keeps_the_newest() {
        let doc = extension_json(
            &entry(),
            &urls(),
            &q(ALL_FLAGS | flag::INCLUDE_LATEST_VERSION_ONLY),
        );
        let versions = doc["versions"].as_array().unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0]["version"], "2.0.0");
    }

    #[test]
    fn the_flags_gate_what_is_included() {
        let bare = extension_json(&entry(), &urls(), &q(0));
        assert_eq!(bare["versions"], json!([]));
        assert_eq!(bare["categories"], json!([]));
        assert!(bare.get("statistics").is_none());

        let full = extension_json(&entry(), &urls(), &q(ALL_FLAGS));
        assert_eq!(full["categories"], json!(["Linters"]));
        assert!(full["statistics"].is_array());
    }

    /// The fidelity guarantee: a key the marketplace sent that this proxy does
    /// not model must survive the round trip.
    #[test]
    fn unmodelled_upstream_fields_survive_rendering() {
        let mut e = entry();
        e.upstream = Some(json!({
            "extensionName": "tool",
            "publisher": { "publisherName": "acme" },
            "deploymentType": 0,
            "someFutureField": { "nested": true },
        }));

        let doc = extension_json(&e, &urls(), &q(ALL_FLAGS));
        assert_eq!(doc["someFutureField"]["nested"], json!(true));
        assert_eq!(doc["deploymentType"], json!(0));
        assert_eq!(
            doc["versions"][0]["assetUri"],
            "https://hub.example.com/proxy/vsx1/vscode/asset/acme/tool/2.0.0",
            "…while the fields the proxy owns are still overwritten"
        );
    }

    /// An upstream `assetUri` must never reach the client — that would route
    /// the download around the cache, the audit trail and the download gate.
    #[test]
    fn an_upstream_asset_uri_is_always_overwritten() {
        let mut e = entry();
        e.upstream = Some(json!({
            "extensionName": "tool",
            "publisher": { "publisherName": "acme" },
            "versions": [{ "version": "2.0.0", "assetUri": "https://marketplace.invalid/x" }],
        }));

        let doc = extension_json(&e, &urls(), &q(ALL_FLAGS));
        let rendered = serde_json::to_string(&doc).unwrap();
        assert!(
            !rendered.contains("marketplace.invalid"),
            "upstream URL leaked: {rendered}"
        );
    }

    #[test]
    fn the_envelope_carries_the_total_count() {
        let doc = query_response_json(&[entry()], 42, &urls(), &q(ALL_FLAGS));
        assert_eq!(
            doc["results"][0]["resultMetadata"][0]["metadataItems"][0]["count"],
            42
        );
        assert_eq!(doc["results"][0]["extensions"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn an_empty_result_is_a_well_formed_envelope() {
        let doc = query_response_json(&[], 0, &urls(), &q(ALL_FLAGS));
        assert_eq!(doc["results"][0]["extensions"], json!([]));
        assert_eq!(
            doc["results"][0]["resultMetadata"][0]["metadataItems"][0]["count"],
            0
        );
    }

    // ── matching ─────────────────────────────────────────────────────────────

    fn query_with(json: serde_json::Value) -> GalleryQuery {
        GalleryQuery::from_request(&serde_json::from_value(json).unwrap())
    }

    #[test]
    fn an_exact_name_lookup_does_not_match_a_prefix() {
        let e = entry();
        assert!(e.matches(&query_with(json!({
            "filters": [{ "criteria": [{ "filterType": 7, "value": "ACME.TOOL" }] }]
        }))));
        assert!(
            !e.matches(&query_with(json!({
                "filters": [{ "criteria": [{ "filterType": 7, "value": "acme" }] }]
            }))),
            "a prefix match here would install the wrong extension"
        );
    }

    #[test]
    fn free_text_matches_name_description_tags_and_categories() {
        let e = entry();
        for text in ["acme", "THING", "linter", "tool"] {
            assert!(
                e.matches(&query_with(json!({
                    "filters": [{ "criteria": [{ "filterType": 10, "value": text }] }]
                }))),
                "should match {text}"
            );
        }
        assert!(!e.matches(&query_with(json!({
            "filters": [{ "criteria": [{ "filterType": 10, "value": "python" }] }]
        }))));
    }

    #[test]
    fn facet_criteria_are_anded() {
        let e = entry();
        assert!(e.matches(&query_with(json!({
            "filters": [{ "criteria": [
                { "filterType": 5, "value": "linters" },
                { "filterType": 1, "value": "acme" }
            ]}]
        }))));
        assert!(
            !e.matches(&query_with(json!({
                "filters": [{ "criteria": [
                    { "filterType": 5, "value": "Linters" },
                    { "filterType": 1, "value": "absent" }
                ]}]
            }))),
            "every criterion must hold"
        );
    }

    #[test]
    fn upstream_versions_decode_their_properties() {
        let v = GalleryVersion::from_upstream(&json!({
            "version": "1.2.3",
            "lastUpdated": "2024-05-01T00:00:00Z",
            "properties": [
                { "key": property_key::ENGINE, "value": "^1.90.0" },
                { "key": property_key::EXTENSION_PACK, "value": "a.b, c.d" },
                { "key": property_key::PRE_RELEASE, "value": "true" },
            ],
        }))
        .unwrap();

        assert_eq!(v.engine.as_deref(), Some("^1.90.0"));
        assert_eq!(v.extension_pack, ["a.b", "c.d"], "comma-separated, trimmed");
        assert!(v.pre_release);
    }

    #[test]
    fn an_upstream_object_missing_its_coordinate_is_skipped() {
        assert!(GalleryEntry::from_upstream(&json!({ "displayName": "no name" })).is_none());
    }
}
