//! Git forges: GitHub, Forgejo/Gitea and GitLab release listings.
//!
//! Three APIs, one document shape — a JSON array of release objects, newest
//! first, each naming its release by `tag_name`. Forgejo is deliberately
//! GitHub-compatible here, and GitLab's `/releases` uses the same field name.
//!
//! A "version" on a forge is a **tag**, and tags are spelled inconsistently:
//! the same release is `1.2.3` in one repository and `v1.2.3` in the next. An
//! operator who blocks `1.2.3` means the release, not the string, so
//! `normalize` strips the prefix on both sides for these kinds.
//!
//! Nothing in the document names a preferred release beyond its position, so
//! there is nothing to repair — dropping the entry is the whole filter.

use serde_json::Value;

use super::BlockedVersions;

/// Remove blocked releases from a forge's release listing.
///
/// Order is preserved: forges serve these newest-first and clients page through
/// them in that order.
pub fn strip_releases(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let Some(releases) = doc.as_array_mut() else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    releases.retain(|r| {
        let Some(tag) = release_tag(r) else {
            // No tag to judge it by; keeping it over-lists, which is the safe
            // direction.
            return true;
        };
        if blocked.contains(tag) {
            removed.push(tag.to_owned());
            false
        } else {
            true
        }
    });
    removed
}

/// The tag a release object names, whichever of the two spellings the forge
/// uses for the field.
fn release_tag(release: &Value) -> Option<&str> {
    release
        .get("tag_name")
        .or_else(|| release.get("tag"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Github,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn releases() -> Value {
        json!([
            { "id": 3, "tag_name": "v2.0.0", "published_at": "2024-03-01T00:00:00Z" },
            { "id": 2, "tag_name": "v1.1.0", "published_at": "2024-02-01T00:00:00Z" },
            { "id": 1, "tag_name": "v1.0.0", "published_at": "2024-01-01T00:00:00Z" }
        ])
    }

    fn tags(doc: &Value) -> Vec<String> {
        doc.as_array()
            .expect("a release array")
            .iter()
            .map(|r| r["tag_name"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn a_blocked_release_leaves_the_listing_and_order_survives() {
        let mut doc = releases();
        let removed = strip_releases(&mut doc, &blocked(&["v1.1.0"]));

        assert_eq!(removed, vec!["v1.1.0".to_owned()]);
        assert_eq!(tags(&doc), ["v2.0.0", "v1.0.0"], "still newest-first");
    }

    /// The same release is tagged `1.2.3` in one repository and `v1.2.3` in the
    /// next; a block must not depend on which habit the operator copied.
    #[test]
    fn a_block_matches_a_tag_with_or_without_its_v_prefix() {
        let mut doc = releases();
        strip_releases(&mut doc, &blocked(&["1.1.0"]));
        assert_eq!(tags(&doc), ["v2.0.0", "v1.0.0"]);

        let mut doc = json!([{ "tag_name": "1.1.0" }]);
        strip_releases(&mut doc, &blocked(&["v1.1.0"]));
        assert_eq!(doc, json!([]));
    }

    /// Forgejo mirrors GitHub's `tag_name`; some Gitea versions answer with
    /// `tag`. Reading either is cheaper than being wrong on one of them.
    #[test]
    fn either_tag_field_spelling_is_understood() {
        let mut doc = json!([{ "tag": "v1.1.0" }, { "tag": "v1.0.0" }]);
        strip_releases(&mut doc, &blocked(&["v1.1.0"]));

        assert_eq!(doc.as_array().unwrap().len(), 1);
    }

    #[test]
    fn blocking_an_absent_release_changes_nothing() {
        let mut doc = releases();
        let before = doc.clone();
        assert!(strip_releases(&mut doc, &blocked(&["v9.9.9"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn blocking_every_release_leaves_an_empty_array() {
        let mut doc = releases();
        strip_releases(&mut doc, &blocked(&["v2.0.0", "v1.1.0", "v1.0.0"]));

        assert_eq!(doc, json!([]));
    }

    #[test]
    fn a_release_with_no_tag_is_kept() {
        let mut doc = json!([{ "id": 1, "name": "untagged draft" }]);
        let before = doc.clone();
        assert!(strip_releases(&mut doc, &blocked(&["v1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_document_is_returned_unchanged() {
        let mut doc = json!({ "message": "Not Found" });
        let before = doc.clone();
        assert!(strip_releases(&mut doc, &blocked(&["v1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }
}
