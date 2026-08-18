//! Maven: `maven-metadata.xml`.
//!
//! `<versioning><versions><version>…</version></versions></versioning>`, plus a
//! `<lastUpdated>` that describes the *document* rather than any one version —
//! so it is not read as a publish time.

use super::{text, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(body) = text(doc) else {
        return UpstreamDetail::default();
    };
    UpstreamDetail {
        versions: versions_in(body)
            .into_iter()
            .map(UpstreamVersion::bare)
            .collect(),
        readmes: Default::default(),
    }
}

/// Every `<version>` inside `<versions>`.
///
/// Tag slicing rather than a parser, matching how `blocking::maven` reads the
/// same document for the same reason: the element is a leaf with no attributes
/// and no namespace in any real `maven-metadata.xml`, and a mis-slice here costs
/// a missing row on a page rather than a wrong listing served to a build.
fn versions_in(xml: &str) -> Vec<String> {
    let Some(block_start) = xml.find("<versions>") else {
        return Vec::new();
    };
    let block = &xml[block_start..];
    let block = match block.find("</versions>") {
        Some(end) => &block[..end],
        None => block,
    };

    let mut out = Vec::new();
    let mut rest = block;
    while let Some(open) = rest.find("<version>") {
        let after = &rest[open + "<version>".len()..];
        let Some(close) = after.find("</version>") else {
            break;
        };
        let value = after[..close].trim();
        if !value.is_empty() {
            out.push(value.to_owned());
        }
        rest = &after[close..];
    }
    out
}
