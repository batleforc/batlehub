//! Composer: p2 metadata.
//!
//! `{"packages": {"vendor/name": [ {version, time}, … ]}}`, optionally in the
//! `composer/2.0` minified encoding where each entry after the first carries
//! only its *differences* from the previous one.

use super::{is_prerelease, json, parse_time, UpstreamDetail, UpstreamVersion};
use crate::entities::MetadataLinks;
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };
    let minified = root
        .get("minified")
        .and_then(|m| m.as_str())
        .is_some_and(|m| m == "composer/2.0");
    let Some(packages) = root.get("packages").and_then(|p| p.as_object()) else {
        return UpstreamDetail::default();
    };

    let mut versions = Vec::new();
    let mut links = None;
    for entries in packages.values() {
        let Some(list) = entries.as_array() else {
            continue;
        };
        // In the minified encoding a field absent from an entry means
        // "unchanged from the previous one", so `time` has to be carried
        // forward or every version after the first would report none.
        let mut carried_time: Option<String> = None;
        for entry in list {
            let time = entry
                .get("time")
                .and_then(|t| t.as_str())
                .map(str::to_owned)
                .or_else(|| minified.then(|| carried_time.clone()).flatten());
            carried_time = time.clone();

            // The first entry that names one wins, and p2 lists newest first —
            // so this is the newest release's answer. In the minified encoding
            // the first entry is the complete one, and a later entry that omits
            // `source` means "unchanged", which is the same answer.
            if links.is_none() {
                links = MetadataLinks::new(
                    entry
                        .get("source")
                        .and_then(|s| s.get("url"))
                        .and_then(|v| v.as_str()),
                    entry.get("homepage").and_then(|v| v.as_str()),
                );
            }

            let Some(version) = entry.get("version").and_then(|v| v.as_str()) else {
                continue;
            };
            versions.push(UpstreamVersion {
                version: version.to_owned(),
                published_at: time.as_deref().and_then(parse_time),
                is_prerelease: is_prerelease(version),
                yanked: false,
                deprecated: None,
            });
        }
    }
    UpstreamDetail {
        versions,
        readmes: Default::default(),
        links,
    }
}
