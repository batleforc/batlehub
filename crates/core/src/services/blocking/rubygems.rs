//! RubyGems: the versions API and the single-gem document.
//!
//! `/api/v1/versions/{name}.json` is an array of version objects keyed by
//! `number`. `/api/v1/gems/{name}.json` is a single object describing the gem
//! *at its newest version* — so it has no list to filter, and hiding a version
//! from it means rebuilding it around a different one.
//!
//! The Marshal indexes (`specs.4.8.gz`, `quick/Marshal.4.8/*`) are deliberately
//! not filtered; [`crate::entities::RegistryKind::listing_filter`] records why.

use serde_json::Value;

use super::{best_latest, BlockedVersions};

/// Remove blocked versions from `/api/v1/versions/{name}.json`.
///
/// The array is newest-first as RubyGems serves it, and stays that way:
/// entries are filtered in place.
pub fn strip_versions(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let Some(items) = doc.as_array_mut() else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    items.retain(|item| {
        let Some(v) = item.get("number").and_then(Value::as_str) else {
            return true;
        };
        if blocked.contains(v) {
            removed.push(v.to_owned());
            false
        } else {
            true
        }
    });
    removed
}

/// Repair `/api/v1/gems/{name}.json` — the document *is* one version, so when
/// that version is blocked it has to be rebuilt around another.
///
/// `available` is every version the *filtered* versions API still lists, which
/// is what the caller already has — so "is this version blocked" is answered by
/// its absence rather than by consulting the blocked set a second time, and the
/// two documents cannot disagree about what is visible.
///
/// The rebuilt document keeps the gem's package-level fields (name, homepage,
/// licences…) and moves `version` to the best survivor; the fields that describe
/// the *release* rather than the gem — checksum, download URL, the version's own
/// dates — are removed, because carrying the hidden release's checksum on a
/// different version would hand a client a hash that will never match what it
/// downloads.
///
/// Returns the version that was hidden, or `None` when nothing needed doing.
pub fn repair_gem(doc: &mut Value, available: &[String]) -> Option<String> {
    let current = doc.get("version").and_then(Value::as_str)?.to_owned();
    if available.contains(&current) {
        return None;
    }

    let obj = doc.as_object_mut()?;
    // Release-scoped fields. Left in place they would describe the hidden
    // release while `version` names a different one.
    for release_field in [
        "sha",
        "gem_uri",
        "spec_sha",
        "metadata",
        "built_at",
        "created_at",
        "version_created_at",
        "version_downloads",
        "requirements",
        "dependencies",
        "prerelease",
    ] {
        obj.remove(release_field);
    }

    match best_latest(available) {
        Some(best) => {
            obj.insert("version".to_owned(), Value::String(best));
        }
        None => {
            // Every version is blocked. Leaving `version` naming the blocked
            // release would advertise it; removing the key leaves a document
            // that describes a gem with no installable release, which is the
            // truth.
            obj.remove("version");
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Rubygems,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn versions_doc() -> Value {
        json!([
            { "number": "7.1.0", "sha": "aaa" },
            { "number": "7.0.0", "sha": "bbb" },
            { "number": "6.1.0", "sha": "ccc" }
        ])
    }

    #[test]
    fn versions_api_drops_the_blocked_entry_and_keeps_order() {
        let mut doc = versions_doc();
        let removed = strip_versions(&mut doc, &blocked(&["7.0.0"]));

        assert_eq!(removed, vec!["7.0.0".to_owned()]);
        let numbers: Vec<&str> = doc
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["number"].as_str().unwrap())
            .collect();
        assert_eq!(numbers, ["7.1.0", "6.1.0"], "still newest-first");
    }

    #[test]
    fn versions_api_blocking_everything_leaves_an_empty_array() {
        let mut doc = versions_doc();
        strip_versions(&mut doc, &blocked(&["7.1.0", "7.0.0", "6.1.0"]));

        assert_eq!(doc, json!([]));
    }

    #[test]
    fn versions_api_blocking_an_absent_version_changes_nothing() {
        let mut doc = versions_doc();
        let before = doc.clone();
        assert!(strip_versions(&mut doc, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_versions_document_is_returned_unchanged() {
        let mut doc = json!({ "not": "an array" });
        let before = doc.clone();
        assert!(strip_versions(&mut doc, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }

    fn gem_doc() -> Value {
        json!({
            "name": "rails",
            "version": "7.1.0",
            "sha": "deadbeef",
            "gem_uri": "https://rubygems.org/gems/rails-7.1.0.gem",
            "homepage_uri": "https://rubyonrails.org",
            "licenses": ["MIT"]
        })
    }

    #[test]
    fn gem_document_moves_to_the_best_surviving_version() {
        let mut doc = gem_doc();
        let removed = repair_gem(&mut doc, &["7.0.0".to_owned(), "6.1.0".to_owned()]);

        assert_eq!(removed, Some("7.1.0".to_owned()));
        assert_eq!(doc["version"], json!("7.0.0"));
        assert_eq!(
            doc["homepage_uri"],
            json!("https://rubyonrails.org"),
            "gem-level fields survive"
        );
    }

    /// The checksum and download URL belonged to the hidden release. Carried
    /// onto a different version they are a hash that can never match.
    #[test]
    fn gem_document_drops_the_hidden_release_s_own_fields() {
        let mut doc = gem_doc();
        repair_gem(&mut doc, &["7.0.0".to_owned()]);

        assert!(doc.get("sha").is_none());
        assert!(doc.get("gem_uri").is_none());
    }

    #[test]
    fn gem_document_naming_a_still_listed_version_is_untouched() {
        let mut doc = gem_doc();
        let before = doc.clone();
        assert!(repair_gem(&mut doc, &["7.1.0".to_owned(), "6.1.0".to_owned()]).is_none());
        assert_eq!(doc, before);
    }

    #[test]
    fn gem_document_with_every_version_blocked_names_none() {
        let mut doc = gem_doc();
        repair_gem(&mut doc, &[]);

        assert!(doc.get("version").is_none());
        assert_eq!(doc["name"], json!("rails"));
    }
}
