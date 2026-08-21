//! NuGet: the flat index.
//!
//! `{"versions": ["1.0.0", …]}` — the document that resolves a version, and a
//! list of strings with nothing else in it.

use super::{json, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };
    let versions = root
        .get("versions")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str())
                .map(UpstreamVersion::bare)
                .collect()
        })
        .unwrap_or_default();
    UpstreamDetail {
        versions,
        readmes: Default::default(),
        // A list of strings has nothing else in it.
        links: None,
    }
}
