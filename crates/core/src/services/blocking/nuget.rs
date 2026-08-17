//! NuGet: the flat index, and the registration pages.
//!
//! Two documents describe the same package and both have to hide a blocked
//! version, for different reasons. The **flat index** is what `dotnet restore`
//! resolves a version range against, so leaving a blocked version in it is what
//! makes the restore pick that version and then be refused. The **registration
//! pages** are what a UI and `dotnet list package` read, so leaving it there
//! advertises an installable version that is not.

use serde_json::Value;

use super::BlockedVersions;

/// Remove blocked versions from a flat-container index —
/// `{"versions": ["1.0.0", …]}`.
///
/// Order is preserved: the flat index is ascending by version and clients rely
/// on that, so entries are filtered in place rather than rebuilt from a set.
pub fn strip_flat_index(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let Some(versions) = doc.get_mut("versions").and_then(Value::as_array_mut) else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    versions.retain(|v| {
        let Some(s) = v.as_str() else {
            // A non-string entry is not something this proxy understands; keep
            // it, because over-listing is the safe direction.
            return true;
        };
        if blocked.contains(s) {
            removed.push(s.to_owned());
            false
        } else {
            true
        }
    });
    removed
}

/// Remove blocked versions from a registration index
/// (`/registration5/{id}/index.json`).
///
/// Leaves live at `items[].items[].catalogEntry.version`, and removing one
/// invalidates three derived fields the client trusts: each page's `count`,
/// `lower` and `upper`, plus the outer `count`. A page emptied entirely is
/// dropped rather than left as an empty range.
///
/// **Pages whose `items` is a URL rather than an inline array are passed
/// through unfiltered**, and the caller warns. Filtering them means one
/// upstream request per page on a metadata path, and the flat index — what
/// actually resolves the version — is filtered either way. The warning is what
/// makes the gap visible if a real upstream serves paged registrations often
/// enough to matter.
///
/// Returns `(removed, saw_paged)`.
pub fn strip_registration(doc: &mut Value, blocked: &BlockedVersions) -> (Vec<String>, bool) {
    let mut removed = Vec::new();
    let mut saw_paged = false;

    let Some(pages) = doc.get_mut("items").and_then(Value::as_array_mut) else {
        return (removed, saw_paged);
    };

    for page in pages.iter_mut() {
        let Some(page_obj) = page.as_object_mut() else {
            continue;
        };
        let Some(leaves) = page_obj.get_mut("items").and_then(Value::as_array_mut) else {
            // `items` absent or a URL string: this page is served separately.
            saw_paged = true;
            continue;
        };

        leaves.retain(|leaf| {
            let Some(v) = leaf_version(leaf) else {
                return true;
            };
            if blocked.contains(v) {
                removed.push(v.to_owned());
                false
            } else {
                true
            }
        });

        let surviving: Vec<String> = leaves
            .iter()
            .filter_map(|l| leaf_version(l).map(str::to_owned))
            .collect();
        let leaf_count = leaves.len();
        page_obj.insert("count".to_owned(), Value::from(leaf_count));
        // `lower`/`upper` bound the page's version range. Recomputed from what
        // survives rather than left alone: a client that trusts `upper` to
        // decide whether this page can contain the version it wants would skip
        // a page whose real contents shrank, or open one that no longer holds
        // anything.
        if let Some(lo) = surviving.first() {
            page_obj.insert("lower".to_owned(), Value::String(lo.clone()));
        }
        if let Some(hi) = surviving.last() {
            page_obj.insert("upper".to_owned(), Value::String(hi.clone()));
        }
    }

    if !removed.is_empty() {
        pages.retain(|p| {
            // An emptied page is removed outright. Kept, it is a range that
            // resolves to nothing, which some clients treat as a fetch failure
            // rather than as an empty result.
            p.get("items")
                .and_then(Value::as_array)
                .is_none_or(|l| !l.is_empty())
        });
        let page_count = pages.len();
        if let Some(obj) = doc.as_object_mut() {
            // The outer `count` is the number of *pages*, not of versions —
            // NuGet's registration index counts its own `items` array, the same
            // way each page counts its leaves.
            obj.insert("count".to_owned(), Value::from(page_count));
        }
    }

    (removed, saw_paged)
}

fn leaf_version(leaf: &Value) -> Option<&str> {
    leaf.get("catalogEntry")
        .and_then(|c| c.get("version"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Nuget,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn flat() -> Value {
        json!({ "versions": ["1.0.0", "1.1.0", "2.0.0"] })
    }

    #[test]
    fn flat_index_drops_the_blocked_version_and_keeps_order() {
        let mut doc = flat();
        let removed = strip_flat_index(&mut doc, &blocked(&["1.1.0"]));

        assert_eq!(removed, vec!["1.1.0".to_owned()]);
        assert_eq!(doc["versions"], json!(["1.0.0", "2.0.0"]));
    }

    /// NuGet folds `1.0.0.0` to `1.0.0`, so a block recorded in the four-part
    /// spelling must still hide the three-part listing.
    #[test]
    fn flat_index_matches_across_nuget_version_spellings() {
        let mut doc = flat();
        strip_flat_index(&mut doc, &blocked(&["1.1.0.0"]));

        assert_eq!(doc["versions"], json!(["1.0.0", "2.0.0"]));
    }

    #[test]
    fn flat_index_blocking_everything_leaves_a_well_formed_empty_list() {
        let mut doc = flat();
        strip_flat_index(&mut doc, &blocked(&["1.0.0", "1.1.0", "2.0.0"]));

        assert_eq!(doc["versions"], json!([]));
    }

    #[test]
    fn flat_index_blocking_an_absent_version_changes_nothing() {
        let mut doc = flat();
        let before = doc.clone();
        assert!(strip_flat_index(&mut doc, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_flat_index_is_returned_unchanged() {
        let mut doc = json!({ "not-versions": [] });
        let before = doc.clone();
        assert!(strip_flat_index(&mut doc, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }

    fn leaf(v: &str) -> Value {
        json!({ "catalogEntry": { "version": v, "id": "pkg" } })
    }

    fn registration() -> Value {
        json!({
            "count": 2,
            "items": [
                { "count": 2, "lower": "1.0.0", "upper": "1.1.0",
                  "items": [leaf("1.0.0"), leaf("1.1.0")] },
                { "count": 1, "lower": "2.0.0", "upper": "2.0.0",
                  "items": [leaf("2.0.0")] }
            ]
        })
    }

    #[test]
    fn registration_removes_the_leaf_and_repairs_the_page_bounds() {
        let mut doc = registration();
        let (removed, paged) = strip_registration(&mut doc, &blocked(&["1.1.0"]));

        assert_eq!(removed, vec!["1.1.0".to_owned()]);
        assert!(!paged);
        let page = &doc["items"][0];
        assert_eq!(page["count"], json!(1));
        assert_eq!(page["lower"], json!("1.0.0"));
        assert_eq!(
            page["upper"],
            json!("1.0.0"),
            "upper must follow the leaf it named out of the page"
        );
    }

    #[test]
    fn registration_drops_a_page_it_emptied() {
        let mut doc = registration();
        strip_registration(&mut doc, &blocked(&["2.0.0"]));

        assert_eq!(doc["items"].as_array().unwrap().len(), 1);
        assert_eq!(doc["count"], json!(1), "the outer count counts pages");
    }

    /// A paged registration is passed through whole, and says so, rather than
    /// being silently half-filtered.
    #[test]
    fn registration_passes_paged_items_through_and_reports_it() {
        let mut doc = json!({
            "count": 1,
            "items": [{ "count": 3, "items": "https://api.nuget.org/v3/registration5/pkg/page/1.json" }]
        });
        let before = doc.clone();
        let (removed, paged) = strip_registration(&mut doc, &blocked(&["1.0.0"]));

        assert!(removed.is_empty());
        assert!(paged, "the caller needs to know the gap was hit");
        assert_eq!(doc, before);
    }

    #[test]
    fn registration_blocking_nothing_present_leaves_counts_alone() {
        let mut doc = registration();
        let before = doc.clone();
        let (removed, _) = strip_registration(&mut doc, &blocked(&["9.9.9"]));

        assert!(removed.is_empty());
        assert_eq!(doc, before, "counts must not be rewritten for no reason");
    }
}
