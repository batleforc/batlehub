//! Print the README support table for `docs/registries/index.md`.
//!
//! Generated from [`RegistryKind::readme_support`] and
//! [`RegistryKind::upstream_detail`], both exhaustive matches a new registry
//! kind cannot be added without answering — for the same reason the
//! listing-filter table is generated from `listing_filter()` (RFC 0006 §4.3,
//! RFC 0007 §4.3).
//!
//! A table claiming coverage that dispatch cannot deliver is the failure RFC
//! 0009 was written about, so the reasons are printed verbatim from the code
//! that decides them rather than restated in prose beside it.
//!
//! Run through `task docs:readme-coverage`; `task docs:readme-coverage:check`
//! fails the build if the committed page has drifted.

use batlehub_core::entities::{ReadmeSupport, RegistryKind, UpstreamDetailSupport};

fn main() {
    println!("| Registry | README source | Per version | Held nowhere here |");
    println!("| --- | --- | --- | --- |");

    for kind in RegistryKind::ALL {
        let (source, per_version) = match kind.readme_support() {
            ReadmeSupport::Metadata => ("the metadata document, already fetched", "yes"),
            ReadmeSupport::MetadataLinked => ("a URL in the metadata, read separately", "yes"),
            ReadmeSupport::Archive => ("a file inside the artifact", "yes"),
            ReadmeSupport::MetadataThenArchive => {
                ("the metadata document, else the artifact", "yes")
            }
            ReadmeSupport::None(reason) => (reason, "—"),
        };

        // What the page can say about a version this instance holds no bytes
        // for. The two accessors answer different halves and both matter: a
        // kind can be askable upstream and still have no README to hand back.
        let unheld = match (kind.upstream_detail(), kind.readme_support()) {
            (UpstreamDetailSupport::None(_), _) => "neither",
            (_, support) if support.answers_for_unheld_versions() => "versions + README",
            _ => "versions only",
        };

        println!(
            "| {} | {source} | {per_version} | {unheld} |",
            kind.as_str()
        );
    }
}
