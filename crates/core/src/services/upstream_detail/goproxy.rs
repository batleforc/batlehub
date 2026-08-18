//! Go: `@v/list`.
//!
//! One version per line, and nothing else. `@latest` carries a timestamp but
//! names one version, so it is not what a version table is built from.

use super::{text, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(body) = text(doc) else {
        return UpstreamDetail::default();
    };
    let versions = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(UpstreamVersion::bare)
        .collect();
    UpstreamDetail {
        versions,
        readmes: Default::default(),
    }
}
