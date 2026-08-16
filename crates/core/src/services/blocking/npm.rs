//! npm: the packument.
//!
//! The first protocol to get listing filtering, and the shape the rest follow —
//! see the module docs on [`super`] for what a listing filter is and why the
//! download gate alone is not enough.

use serde_json::{Map, Value};

use super::{best_latest, BlockedVersions};

/// Strip every blocked version from an npm packument, in place, and repair the
/// `dist-tags` that pointed at them.
///
/// Returns the versions actually removed (those present in the document), so a
/// caller can log or count them; an empty return means the document was already
/// free of blocked versions.
///
/// What "repair" means for `dist-tags` is the part that decides whether
/// `npm install lodash` succeeds:
///
/// - `latest` is **recomputed** to the highest remaining version, because a
///   packument whose `latest` names a version absent from `versions` is
///   malformed and npm errors on it. Highest *stable* wins; if only
///   pre-releases survive, the highest of those is used, matching the local
///   registry's own rule (`latest_stable_or_newest`).
/// - Any **other** tag (`next`, `beta`, a team's own tag) pointing at a blocked
///   version is **dropped**, not repointed. A tag is a deliberate label on one
///   specific release, and silently moving it to a different release would
///   misrepresent the publisher's intent — where `latest` has a defined meaning
///   ("the newest thing you should install") that survives recomputation.
///
/// `time` entries are removed alongside their versions so the document stays
/// internally consistent; `time.created`/`time.modified` are left alone, being
/// package-level rather than per-version.
pub fn strip_packument(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    if blocked.is_empty() {
        return Vec::new();
    }
    let Some(obj) = doc.as_object_mut() else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    if let Some(versions) = obj.get_mut("versions").and_then(Value::as_object_mut) {
        // Iterating the *document's* keys rather than the blocked set is what
        // lets a block recorded in one spelling hide a version listed in
        // another: `contains` compares normalised forms, and the key removed is
        // the one the document actually uses.
        let hit: Vec<String> = versions
            .keys()
            .filter(|v| blocked.contains(v))
            .cloned()
            .collect();
        for v in hit {
            versions.remove(&v);
            removed.push(v);
        }
    }
    if removed.is_empty() {
        return removed;
    }

    if let Some(time) = obj.get_mut("time").and_then(Value::as_object_mut) {
        for v in &removed {
            time.remove(v);
        }
    }

    let surviving: Vec<String> = obj
        .get("versions")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    if let Some(tags) = obj.get_mut("dist-tags").and_then(Value::as_object_mut) {
        repair_dist_tags(tags, blocked, &surviving);
    }

    removed
}

/// Drop tags naming a blocked version, then re-point `latest` at the best
/// surviving version. See [`strip_packument`] for why `latest` is treated
/// differently from every other tag.
fn repair_dist_tags(
    tags: &mut Map<String, Value>,
    blocked: &BlockedVersions,
    surviving: &[String],
) {
    tags.retain(|tag, target| {
        let points_at_blocked = target.as_str().is_some_and(|t| blocked.contains(t));
        // `latest` is repaired below rather than dropped: a packument with no
        // `latest` at all makes npm fall back to the highest version it can
        // find, which is exactly the resolution decision we are trying to own.
        !points_at_blocked || tag == "latest"
    });

    let Some(best) = best_latest(surviving) else {
        // Every version was blocked. A `latest` pointing at something that no
        // longer exists is worse than no tag, and the caller's listing is empty
        // anyway.
        tags.remove("latest");
        return;
    };

    let latest_is_stale = tags
        .get("latest")
        .and_then(Value::as_str)
        .is_none_or(|t| blocked.contains(t));
    if latest_is_stale {
        tags.insert("latest".to_owned(), Value::String(best));
    }
}

/// Point every `versions.*.dist.tarball` at this proxy's download route,
/// matching the URL shape the local registry already serves
/// (`{base}/{name}/{version}/tarball`).
///
/// Lives beside the strip it pairs with rather than in the proxy service: every
/// protocol rewrites its download URLs differently — Composer's `dist.url`,
/// PyPI's `<a href>`, cargo's `dl` — and each rewrite is as much npm (or
/// Composer, or PyPI) vocabulary as the strip is.
///
/// Served unrewritten, the upstream document points every download at the
/// upstream CDN: past this proxy's cache, its audit trail, and the download-time
/// gate that is the block's other half.
pub fn rewrite_tarball_urls(doc: &mut Value, public_base: &str, name: &str) {
    let base = public_base.trim_end_matches('/');
    let name = super::encode_package_segment(name);
    let Some(versions) = doc.get_mut("versions").and_then(Value::as_object_mut) else {
        return;
    };
    for (version, meta) in versions.iter_mut() {
        let Some(obj) = meta.as_object_mut() else {
            continue;
        };
        let dist = obj.entry("dist").or_insert_with(|| serde_json::json!({}));
        if let Some(d) = dist.as_object_mut() {
            d.insert(
                "tarball".to_owned(),
                Value::String(format!("{base}/{name}/{version}/tarball")),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Npm,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn lodash() -> Value {
        json!({
            "name": "lodash",
            "dist-tags": { "latest": "4.17.21", "next": "5.0.0-beta.1" },
            "versions": {
                "4.17.19": {"version": "4.17.19"},
                "4.17.20": {"version": "4.17.20"},
                "4.17.21": {"version": "4.17.21"},
                "5.0.0-beta.1": {"version": "5.0.0-beta.1"}
            },
            "time": {
                "created": "2012-04-23T16:37:11.912Z",
                "4.17.19": "2020-07-08T18:56:29.158Z",
                "4.17.20": "2020-08-13T16:53:54.014Z",
                "4.17.21": "2021-02-20T15:42:16.891Z",
                "5.0.0-beta.1": "2021-03-01T00:00:00.000Z"
            }
        })
    }

    /// The case in the brief: blocking the newest stable must move `latest`
    /// back to the newest *allowed* stable, not to the newer pre-release.
    #[test]
    fn blocking_latest_repoints_to_previous_stable() {
        let mut doc = lodash();
        let removed = strip_packument(&mut doc, &blocked(&["4.17.21"]));

        assert_eq!(removed, vec!["4.17.21".to_owned()]);
        assert_eq!(doc["dist-tags"]["latest"], json!("4.17.20"));
        assert!(doc["versions"].get("4.17.21").is_none());
        assert!(doc["versions"].get("4.17.20").is_some());
        assert!(doc["time"].get("4.17.21").is_none());
        // Package-level time keys survive.
        assert!(doc["time"].get("created").is_some());
    }

    #[test]
    fn tags_other_than_latest_pointing_at_a_blocked_version_are_dropped() {
        let mut doc = lodash();
        strip_packument(&mut doc, &blocked(&["5.0.0-beta.1"]));

        assert!(doc["dist-tags"].get("next").is_none());
        // `latest` was not pointing at the blocked version, so it stays put.
        assert_eq!(doc["dist-tags"]["latest"], json!("4.17.21"));
    }

    #[test]
    fn latest_falls_back_to_a_prerelease_when_no_stable_survives() {
        let mut doc = lodash();
        strip_packument(&mut doc, &blocked(&["4.17.19", "4.17.20", "4.17.21"]));

        assert_eq!(doc["dist-tags"]["latest"], json!("5.0.0-beta.1"));
    }

    #[test]
    fn blocking_every_version_leaves_no_latest_tag() {
        let mut doc = lodash();
        strip_packument(
            &mut doc,
            &blocked(&["4.17.19", "4.17.20", "4.17.21", "5.0.0-beta.1"]),
        );

        assert_eq!(doc["versions"].as_object().unwrap().len(), 0);
        assert!(doc["dist-tags"].get("latest").is_none());
    }

    #[test]
    fn no_blocks_leaves_the_document_untouched() {
        let mut doc = lodash();
        let before = doc.clone();
        let removed = strip_packument(
            &mut doc,
            &BlockedVersions::new(RegistryKind::Npm, Vec::new()),
        );

        assert!(removed.is_empty());
        assert_eq!(doc, before);
    }

    /// A block on a version the upstream does not serve must not disturb the
    /// document — in particular it must not trigger a `latest` recomputation.
    #[test]
    fn blocking_an_absent_version_changes_nothing() {
        let mut doc = lodash();
        let before = doc.clone();
        let removed = strip_packument(&mut doc, &blocked(&["9.9.9"]));

        assert!(removed.is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn semver_ordering_beats_lexicographic_when_choosing_latest() {
        // "4.17.9" > "4.17.10" lexicographically, but not by semver.
        let mut doc = json!({
            "dist-tags": { "latest": "4.17.21" },
            "versions": {
                "4.17.9": {}, "4.17.10": {}, "4.17.21": {}
            }
        });
        strip_packument(&mut doc, &blocked(&["4.17.21"]));

        assert_eq!(doc["dist-tags"]["latest"], json!("4.17.10"));
    }

    #[test]
    fn handles_a_document_with_no_dist_tags_or_time() {
        let mut doc = json!({ "versions": { "1.0.0": {}, "2.0.0": {} } });
        let removed = strip_packument(&mut doc, &blocked(&["2.0.0"]));

        assert_eq!(removed, vec!["2.0.0".to_owned()]);
        assert!(doc["versions"].get("2.0.0").is_none());
    }

    #[test]
    fn non_object_document_is_left_alone() {
        let mut doc = json!("not a packument");
        assert!(strip_packument(&mut doc, &blocked(&["1.0.0"])).is_empty());
    }

    #[test]
    fn tarball_urls_point_back_at_this_proxy() {
        let mut doc = lodash();
        rewrite_tarball_urls(&mut doc, "https://hub.example.com/proxy/npm1/", "lodash");

        assert_eq!(
            doc["versions"]["4.17.20"]["dist"]["tarball"],
            json!("https://hub.example.com/proxy/npm1/lodash/4.17.20/tarball"),
            "the trailing slash on the base must not double up"
        );
    }

    /// A scoped name interpolated raw would split into two path segments and
    /// 404 every scoped dependency in the document.
    #[test]
    fn scoped_names_stay_one_path_segment() {
        let mut doc = json!({ "versions": { "3.0.0": {} } });
        rewrite_tarball_urls(&mut doc, "https://h/proxy/npm1", "@vue/cli");

        assert_eq!(
            doc["versions"]["3.0.0"]["dist"]["tarball"],
            json!("https://h/proxy/npm1/@vue%2Fcli/3.0.0/tarball")
        );
    }
}
