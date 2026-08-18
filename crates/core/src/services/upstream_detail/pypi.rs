//! PyPI: the PEP 691 JSON simple page.
//!
//! `versions` (PEP 700) when the index publishes it, and the distinct versions
//! parsed out of `files[].filename` when it does not — which is still every
//! index in use, so the fallback is the common path rather than the edge.
//!
//! The *README* is not here: `info.description` lives in
//! `/pypi/{name}/{version}/json`, one request per version, so the panel fetches
//! it on selection rather than filling the whole table (RFC 0007 open
//! question 7).

use std::collections::BTreeSet;

use super::{json, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };

    // PEP 700's `versions` is the index's own answer and includes versions with
    // no files left — a version whose only wheel was deleted still exists.
    if let Some(list) = root.get("versions").and_then(|v| v.as_array()) {
        let versions = list
            .iter()
            .filter_map(|v| v.as_str())
            .map(UpstreamVersion::bare)
            .collect::<Vec<_>>();
        if !versions.is_empty() {
            return UpstreamDetail {
                versions,
                readmes: Default::default(),
            };
        }
    }

    // A `BTreeSet` because one version has many files — a wheel per platform
    // plus an sdist — and the table wants one row each, in a stable order.
    let mut seen = BTreeSet::new();
    if let Some(files) = root.get("files").and_then(|f| f.as_array()) {
        for file in files {
            let Some(name) = file.get("filename").and_then(|n| n.as_str()) else {
                continue;
            };
            if let Some(version) = version_from_filename(name) {
                seen.insert(version.to_owned());
            }
        }
    }
    UpstreamDetail {
        versions: seen.into_iter().map(UpstreamVersion::bare).collect(),
        readmes: Default::default(),
    }
}

/// The version segment of a distribution filename.
///
/// The same rule `blocking::pypi` applies to the same filenames, kept here
/// rather than shared because the two modules are allowed to diverge: this one
/// may gain shapes the filter must not, and a shared helper would make that a
/// cross-module change.
fn version_from_filename(filename: &str) -> Option<&str> {
    let stem = filename
        .strip_suffix(".whl")
        .or_else(|| filename.strip_suffix(".tar.gz"))
        .or_else(|| filename.strip_suffix(".tar.bz2"))
        .or_else(|| filename.strip_suffix(".zip"))
        .or_else(|| filename.strip_suffix(".egg"))?;

    stem.split('-')
        .skip(1)
        .find(|part| part.starts_with(|c: char| c.is_ascii_digit()))
}
