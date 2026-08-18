//! RubyGems: the versions API.
//!
//! A JSON array of `{number, created_at, prerelease}` — the one protocol here
//! that states pre-release status rather than leaving it to be inferred from
//! the version string.

use super::{is_prerelease, json, parse_time, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };
    let Some(items) = root.as_array() else {
        return UpstreamDetail::default();
    };
    let versions = items
        .iter()
        .filter_map(|item| {
            let version = item.get("number")?.as_str()?.to_owned();
            Some(UpstreamVersion {
                published_at: item
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .and_then(parse_time),
                // The document's own answer wins over the string heuristic:
                // RubyGems' pre-release rule is "contains a letter", which
                // `1.0.0.beta` satisfies and `contains('-')` does not.
                is_prerelease: item
                    .get("prerelease")
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| is_prerelease(&version)),
                version,
                yanked: false,
                deprecated: None,
            })
        })
        .collect();
    UpstreamDetail {
        versions,
        readmes: Default::default(),
    }
}
