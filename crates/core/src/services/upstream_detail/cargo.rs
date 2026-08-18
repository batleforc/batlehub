//! cargo: the sparse index.
//!
//! One JSON object per line, carrying `vers` and `yanked`. No publish times:
//! the index is what cargo resolves against and carries nothing else, so the
//! page renders those cells as *unknown* rather than inventing them.

use super::{is_prerelease, text, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(body) = text(doc) else {
        return UpstreamDetail::default();
    };
    let versions = body
        .lines()
        .filter(|line| !line.trim().is_empty())
        // A line that does not parse contributes nothing, which is the
        // over-listing direction inverted: fewer rows, never a wrong one.
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter_map(|entry| {
            let version = entry.get("vers")?.as_str()?.to_owned();
            Some(UpstreamVersion {
                is_prerelease: is_prerelease(&version),
                // cargo's own withdrawal mark, not this instance's policy —
                // blocks are applied on top so the two can be told apart.
                yanked: entry
                    .get("yanked")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                version,
                published_at: None,
                deprecated: None,
            })
        })
        .collect();
    UpstreamDetail {
        versions,
        readmes: Default::default(),
    }
}
