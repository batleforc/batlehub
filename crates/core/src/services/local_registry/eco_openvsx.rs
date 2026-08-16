use chrono::{DateTime, Utc};

use super::{CoreError, Identity, LocalRegistryService, PublishedPackage};

/// One published VS Code extension version, decoded from the per-version
/// `index_metadata` written at publish time.
///
/// The web layer renders both client protocols from this — the VS Code gallery
/// (`extensionquery`) and the OpenVSX REST API — so it carries everything
/// either of them shows, not just what an install needs. An editor that gets a
/// bare id and version renders a blank tile with no description, no categories
/// and no engine constraint, which is a working install and a useless
/// marketplace.
#[derive(Debug, Clone)]
pub struct OpenVsxExtensionVersion {
    /// `{publisher}.{name}`, the id the whole ecosystem addresses by.
    pub extension_id: String,
    /// The `{publisher}` half, split out because both protocols report it
    /// separately.
    pub publisher: String,
    /// The `{name}` half.
    pub name: String,
    pub version: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    /// The `engines.vscode` range — the editor refuses to install a version
    /// whose engine it does not satisfy, so an absent value is not the same as
    /// a permissive one and stays `None` rather than becoming `*`.
    pub engine: Option<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub extension_pack: Vec<String>,
    pub dependencies: Vec<String>,
    /// Path of the icon *inside* the VSIX, when the manifest declares one.
    pub icon_path: Option<String>,
    /// A pre-release in the VS Code sense (`--pre-release` at package time),
    /// which the editor hides unless the user opted in.
    pub pre_release: bool,
    /// SHA-256 hex of the VSIX bytes.
    pub checksum: String,
    pub published_at: DateTime<Utc>,
}

impl OpenVsxExtensionVersion {
    fn from_published(pkg: &PublishedPackage) -> Self {
        let m = &pkg.index_metadata;
        let s = |key: &str| {
            m.get(key)
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let list = |key: &str| {
            m.get(key)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .filter(|x| !x.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };

        // `publisher` is recorded at publish time, but an extension published
        // before that field existed still has a well-formed id to split.
        let publisher = s("publisher")
            .or_else(|| pkg.name.split('.').next().map(str::to_owned))
            .unwrap_or_default();
        let name = pkg
            .name
            .split_once('.')
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_else(|| pkg.name.clone());

        Self {
            extension_id: pkg.name.clone(),
            publisher,
            name,
            version: pkg.version.clone(),
            display_name: s("displayName"),
            description: s("description"),
            engine: s("engine"),
            categories: list("categories"),
            tags: list("keywords"),
            extension_pack: list("extensionPack"),
            dependencies: list("extensionDependencies"),
            icon_path: s("icon"),
            pre_release: m
                .get("preRelease")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            checksum: pkg.checksum.clone(),
            published_at: pkg.published_at,
        }
    }

    /// Whether `query` matches this extension, for the free-text search both
    /// protocols expose.
    ///
    /// Substring, case-insensitive, over the fields a user would type: the id,
    /// the display name, the description and the keywords. Deliberately not a
    /// ranked search — a local registry holds tens of extensions, not tens of
    /// thousands, and a relevance model nobody can explain is worse than an
    /// obvious one.
    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        let q = query.to_lowercase();
        let hit = |s: &str| s.to_lowercase().contains(&q);
        hit(&self.extension_id)
            || self.display_name.as_deref().is_some_and(hit)
            || self.description.as_deref().is_some_and(hit)
            || self.tags.iter().any(|t| hit(t))
    }
}

impl LocalRegistryService {
    /// All visible, non-yanked versions of one extension, oldest first.
    ///
    /// Goes through `load_visible_versions_or_not_found`, the chokepoint that
    /// already applies `filter_unlisted → filter_blocked → filter_for_identity`
    /// — so an administratively blocked version is absent from every document
    /// the gallery renders without the gallery having to know about blocking.
    ///
    /// Yanked versions are dropped here: neither the VS Code gallery nor the
    /// OpenVSX API has a representation for "exists but withdrawn", so listing
    /// one would offer the editor a version it will then refuse to serve.
    pub async fn get_openvsx_versions(
        &self,
        registry: &str,
        extension_id: &str,
        identity: &Identity,
    ) -> Result<Vec<OpenVsxExtensionVersion>, CoreError> {
        let versions = self
            .load_visible_versions_or_not_found(registry, extension_id, identity, "extension")
            .await?;
        let versions: Vec<_> = versions
            .iter()
            .filter(|p| !p.yanked)
            .map(OpenVsxExtensionVersion::from_published)
            .collect();
        if versions.is_empty() {
            return Err(CoreError::NotFound(format!(
                "extension '{extension_id}' not found in local registry '{registry}'"
            )));
        }
        Ok(versions)
    }

    /// The newest visible version of every extension in `registry` matching
    /// `query`, for the search both protocols expose.
    ///
    /// Packages whose visibility check denies `identity` are skipped rather
    /// than surfaced as an error: a search should show what the caller may see,
    /// and a `403` from one private extension must not blank the whole result
    /// set. Results are sorted by id so paging is stable across requests.
    pub async fn get_openvsx_extensions(
        &self,
        registry: &str,
        query: &str,
        identity: &Identity,
    ) -> Result<Vec<OpenVsxExtensionVersion>, CoreError> {
        let names = self.backend.list_package_names(registry).await?;
        let mut hits = Vec::new();
        for name in names {
            let versions = match self.load_visible_versions(registry, &name, identity).await {
                Ok(v) => v,
                Err(CoreError::AccessDenied(_)) => continue,
                Err(e) => return Err(e),
            };
            if let Some(best) = Self::newest_openvsx(&versions) {
                if best.matches(query) {
                    hits.push(best);
                }
            }
        }
        hits.sort_by(|a, b| a.extension_id.cmp(&b.extension_id));
        Ok(hits)
    }

    /// Newest version by publish date.
    ///
    /// Extension versions are conventionally semver, but nothing enforces it
    /// and `1.10.0` sorts below `1.9.0` lexicographically — the publish
    /// timestamp is the ordering the registry actually knows to be true. Same
    /// reasoning as `newest_jetbrains_match`.
    fn newest_openvsx(versions: &[PublishedPackage]) -> Option<OpenVsxExtensionVersion> {
        versions
            .iter()
            .filter(|p| !p.yanked)
            .max_by_key(|p| p.published_at)
            .map(OpenVsxExtensionVersion::from_published)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::Visibility;

    fn published(name: &str, version: &str, meta: serde_json::Value) -> PublishedPackage {
        PublishedPackage {
            registry: "vsx".to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            checksum: "abc".to_owned(),
            yanked: false,
            deprecated: false,
            deprecation_message: None,
            unlisted: false,
            index_metadata: meta,
            published_at: Utc::now(),
            published_by: None,
            signature_bytes: None,
            signature_type: None,
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn the_extension_id_splits_into_publisher_and_name() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "ms-python.python",
            "1.0.0",
            serde_json::json!({}),
        ));
        assert_eq!(v.publisher, "ms-python");
        assert_eq!(v.name, "python");
        assert_eq!(v.extension_id, "ms-python.python");
    }

    /// An extension name may itself contain dots; only the first separates the
    /// publisher, matching the adapters' `parse_id`.
    #[test]
    fn only_the_first_dot_separates_the_publisher() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "acme.my.extension",
            "1.0.0",
            serde_json::json!({}),
        ));
        assert_eq!(v.publisher, "acme");
        assert_eq!(v.name, "my.extension");
    }

    #[test]
    fn manifest_fields_are_decoded() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "acme.tool",
            "2.0.0",
            serde_json::json!({
                "displayName": "Acme Tool",
                "description": "Does the thing",
                "engine": "^1.85.0",
                "categories": ["Linters", "Other"],
                "keywords": ["acme", "lint"],
                "extensionPack": ["acme.other"],
                "extensionDependencies": ["ms-python.python"],
                "icon": "images/icon.png",
                "preRelease": true
            }),
        ));
        assert_eq!(v.display_name.as_deref(), Some("Acme Tool"));
        assert_eq!(v.engine.as_deref(), Some("^1.85.0"));
        assert_eq!(v.categories, ["Linters", "Other"]);
        assert_eq!(v.tags, ["acme", "lint"]);
        assert_eq!(v.extension_pack, ["acme.other"]);
        assert_eq!(v.dependencies, ["ms-python.python"]);
        assert_eq!(v.icon_path.as_deref(), Some("images/icon.png"));
        assert!(v.pre_release);
    }

    /// An extension published before the richer metadata existed must still
    /// decode — it renders plainly rather than failing the whole listing.
    #[test]
    fn a_bare_index_metadata_still_decodes() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "acme.tool",
            "1.0.0",
            serde_json::json!({ "id": "acme.tool", "version": "1.0.0" }),
        ));
        assert_eq!(v.publisher, "acme");
        assert!(v.display_name.is_none());
        assert!(v.categories.is_empty());
        assert!(!v.pre_release);
    }

    /// Empty strings in the manifest are absent values, not empty labels.
    #[test]
    fn empty_strings_are_treated_as_absent() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "acme.tool",
            "1.0.0",
            serde_json::json!({ "displayName": "", "description": "", "categories": ["", "Other"] }),
        ));
        assert!(v.display_name.is_none());
        assert!(v.description.is_none());
        assert_eq!(v.categories, ["Other"]);
    }

    #[test]
    fn search_matches_id_display_name_description_and_keywords() {
        let v = OpenVsxExtensionVersion::from_published(&published(
            "acme.tool",
            "1.0.0",
            serde_json::json!({
                "displayName": "Acme Tool",
                "description": "Formats YAML",
                "keywords": ["linting"]
            }),
        ));
        assert!(v.matches("acme"), "id");
        assert!(v.matches("TOOL"), "case-insensitive");
        assert!(v.matches("yaml"), "description");
        assert!(v.matches("lint"), "keyword substring");
        assert!(v.matches(""), "an empty query matches everything");
        assert!(!v.matches("python"));
    }
}
