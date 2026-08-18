//! Reading an upstream's own listing document for the console's discovery read
//! (RFC 0007 §5.5).
//!
//! Laid out like [`crate::services::blocking`], and for the same reasons: one
//! file per protocol, each a **pure function** over a `VersionDocument`, one
//! `dispatch` call site, and coverage declared by an exhaustive match on
//! `RegistryKind`. `blocking` already proves `core` can read these documents
//! without knowing what HTTP is.
//!
//! Two modules parsing the same documents to different structs would drift, so
//! this reads what both halves of the page need — the version list *and*, for
//! the kinds whose listing carries it, the README — and `blocking`'s filters
//! stay where they are.
//!
//! **Everything here fails open in the safe direction**, which for a read path
//! means *fewer rows* rather than a broken page: an unparseable document is
//! warned about and contributes nothing, a missing field yields `None` rather
//! than a guess, and no failure of this path can make an artifact retrievable —
//! the download gate re-checks the concrete coordinate as it always has.

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::entities::{MetadataReadme, RegistryKind};
use crate::ports::VersionDocument;

pub use coordinator::UpstreamDetailCoordinator;

mod cargo;
mod composer;
pub mod coordinator;
mod goproxy;
mod maven;
mod npm;
mod nuget;
mod pypi;
mod rubygems;
mod terraform;

/// One version an upstream knows about, described only by what its listing
/// document actually says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpstreamVersion {
    pub version: String,
    /// When the upstream published it, when the document carries that. `None`
    /// where the protocol has no such field — a NuGet flat index is a list of
    /// strings — rather than a guess.
    pub published_at: Option<DateTime<Utc>>,
    pub is_prerelease: bool,
    /// The upstream's own withdrawal mark, where the protocol has one: cargo's
    /// `yanked`. **Not this instance's policy** — blocks are applied on top, by
    /// the handler, so the two can be told apart on the page.
    pub yanked: bool,
    /// The upstream's own deprecation notice, where the protocol has one: npm's
    /// `deprecated` string.
    pub deprecated: Option<String>,
}

impl UpstreamVersion {
    /// A version with nothing known about it but its number.
    ///
    /// The honest shape for the protocols whose listing is a bare list, and the
    /// reason the page renders those cells as *unknown* rather than as `0`.
    pub fn bare(version: impl Into<String>) -> Self {
        let version = version.into();
        Self {
            is_prerelease: is_prerelease(&version),
            version,
            published_at: None,
            yanked: false,
            deprecated: None,
        }
    }
}

/// What one upstream listing document says about a package.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpstreamDetail {
    pub versions: Vec<UpstreamVersion>,
    /// Per-version READMEs, for the kinds whose listing document carries the
    /// text: npm's packument does, a cargo sparse index does not.
    pub readmes: HashMap<String, MetadataReadme>,
}

/// Read `doc` as `kind`'s listing document.
///
/// The single call site. A kind with no reader here returns an empty detail and
/// warns — the page then answers from local rows, which is the degradation this
/// whole path is designed around.
pub fn dispatch(kind: RegistryKind, doc: &VersionDocument) -> UpstreamDetail {
    match kind {
        RegistryKind::Npm => npm::read(doc),
        RegistryKind::Pypi => pypi::read(doc),
        RegistryKind::Cargo => cargo::read(doc),
        RegistryKind::Nuget => nuget::read(doc),
        RegistryKind::Goproxy => goproxy::read(doc),
        RegistryKind::Maven => maven::read(doc),
        RegistryKind::Composer => composer::read(doc),
        RegistryKind::Rubygems => rubygems::read(doc),
        RegistryKind::Terraform => terraform::read(doc),
        other => {
            // Reachable only through a bug: `RegistryKind::upstream_detail()`
            // answers `Document(_)` for exactly the kinds above, and the drift
            // test below refuses any other combination. Warned rather than
            // panicked, because this is a page-render path.
            tracing::warn!(
                kind = %other,
                "no upstream-detail reader for this registry kind; answering from local rows"
            );
            UpstreamDetail::default()
        }
    }
}

/// Whether this kind's *listing* document is also where its README lives.
///
/// npm's packument is both: it lists the versions and carries the text, so if
/// the listing read found none, resolving a version would re-fetch the same
/// document and find the same nothing. PyPI's simple page lists versions and
/// carries no description at all, and the extension galleries answer by query —
/// for those, a per-version resolve is a different request with a different
/// answer, and worth making.
///
/// Exhaustive with no wildcard arm, so a kind added to `dispatch` has to say
/// which it is rather than silently costing an extra upstream request per
/// version viewed.
pub fn listing_carries_readmes(kind: RegistryKind) -> bool {
    match kind {
        RegistryKind::Npm => true,
        RegistryKind::Pypi
        | RegistryKind::Cargo
        | RegistryKind::Nuget
        | RegistryKind::Goproxy
        | RegistryKind::Maven
        | RegistryKind::Composer
        | RegistryKind::Rubygems
        | RegistryKind::Terraform
        | RegistryKind::Conda
        | RegistryKind::Openvsx
        | RegistryKind::VscodeMarketplace
        | RegistryKind::JetbrainsMarketplace
        | RegistryKind::Github
        | RegistryKind::Gitlab
        | RegistryKind::Forgejo
        | RegistryKind::Deb
        | RegistryKind::Rpm
        | RegistryKind::Pacman
        | RegistryKind::Jetbrains
        | RegistryKind::Generic => false,
    }
}

/// A semver-ish pre-release check, matching what the detail page already does.
///
/// Deliberately the same crude rule the existing version table uses (`contains
/// '-'`) rather than a `semver` parse: the two lists sit in one table and a row
/// that sorted differently depending on where it came from would be a visible
/// inconsistency with no upside.
pub(crate) fn is_prerelease(version: &str) -> bool {
    version.contains('-')
}

/// Parse an RFC 3339 timestamp, ignoring anything that is not one.
pub(crate) fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// The JSON body, or a warning and nothing.
pub(crate) fn json(doc: &VersionDocument) -> Option<&serde_json::Value> {
    let body = doc.body.as_json();
    if body.is_none() {
        tracing::warn!("upstream detail expected a JSON document and got text");
    }
    body
}

/// The text body, or a warning and nothing.
pub(crate) fn text(doc: &VersionDocument) -> Option<&str> {
    let body = doc.body.as_text();
    if body.is_none() {
        tracing::warn!("upstream detail expected a text document and got JSON");
    }
    body
}

#[cfg(test)]
mod tests;
