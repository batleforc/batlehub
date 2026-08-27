//! Where a package archive's own manifest lives, and how to be sure you read
//! *that* one.
//!
//! # The defect this exists to remove
//!
//! Every extractor here answered "which entry is the manifest?" with a suffix
//! test and took the **first** entry that passed: `ends_with("pom.xml")`,
//! `ends_with(".nuspec")`, a bare `Cargo.toml`/`PKG-INFO` filename at any depth.
//! Archive order is chosen by whoever built the archive, so a second manifest
//! placed earlier in the file decided the answer. Verified against the
//! pre-change code:
//!
//! - a `.crate` holding `aaa/vendor/Cargo.toml` (`license = "MIT"`) before
//!   `evil-1.0.0/Cargo.toml` (`license = "GPL-3.0-only"`) reported **MIT**;
//! - a jar holding `a/decoypom.xml` before `META-INF/maven/…/pom.xml` reported
//!   the decoy — `ends_with("pom.xml")` matched a file not even named
//!   `pom.xml`.
//!
//! `npm.rs` was the only one that constrained position (`depth != 2`), and even
//! that was not enough on its own: `aaa/package.json` sits at the same depth as
//! `package/package.json` and still won on order.
//!
//! # Why it matters
//!
//! This read produces two things: the licence [`LicenseGateRule`] evaluates,
//! and the `license`/dependency fields of the SPDX and CycloneDX documents
//! attached on release. Neither is a gate on bytes — the publisher of a package
//! authors its real manifest anyway, so a decoy grants no licence they could not
//! simply declare. What it grants is **disagreement**: the gate and the
//! attested SBOM report one licence while `cargo metadata`, Maven, `pip`, and
//! any auditor reading the same archive report another. A control that can be
//! made to disagree with the archive it is reading is not a control.
//!
//! # The rule
//!
//! Match the manifest's **canonical location** for its ecosystem, and require
//! **exactly one** entry to match. Anchoring alone does not close it: a `.crate`
//! may hold `aaa/Cargo.toml` at the same depth as the real one, a `.nupkg` may
//! hold two root `.nuspec` files, a wheel two `*.dist-info/METADATA`. Requiring
//! a sole match is what takes the *choice* away — a planted second manifest now
//! yields "unknown", never an answer of the attacker's picking.
//!
//! "Unknown" is the state `license_gate` is built around: `allow_unknown`
//! defaults to `true`, so the default posture is unchanged, and an operator who
//! set it to `false` asked for exactly this conservatism. What must never happen
//! is answering *permissive* over a condition that was never observed — which is
//! what picking the first of several did.
//!
//! [`LicenseGateRule`]: batlehub_core::rules::LicenseGateRule

/// An archive entry's path segments, ignoring empty ones.
///
/// A trailing or doubled `/` must not change an entry's apparent depth, or
/// `package//package.json` reads as nested and `evil/` as a file.
pub(super) fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// The one item, or `None` if there were none **or more than one**.
///
/// The second half is the point: see the module docs. `None` means "this
/// archive does not name its manifest unambiguously", which every caller
/// already handles as "unknown".
pub(super) fn sole<T>(items: impl IntoIterator<Item = T>) -> Option<T> {
    let mut it = items.into_iter();
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

/// `{name}-{version}/Cargo.toml` — a `.crate` wraps everything in one directory.
pub(super) fn is_crate_manifest(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 2 && segs[1] == "Cargo.toml"
}

/// `package/package.json` — npm's tarballs are prefixed `package/`.
pub(super) fn is_npm_manifest(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 2 && segs[1] == "package.json"
}

/// `META-INF/maven/{groupId}/{artifactId}/pom.xml`, the location Maven itself
/// writes and reads.
///
/// An uber/shaded jar carries one of these per bundled dependency and there is
/// no field in the archive saying which is the jar's own, so it matches several
/// and [`sole`] reports unknown. That is a change from picking whichever came
/// first, which on a shaded jar was some *dependency's* licence reported as the
/// artifact's.
pub(super) fn is_maven_manifest(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 5 && segs[0] == "META-INF" && segs[1] == "maven" && segs[4] == "pom.xml"
}

/// `{id}.nuspec` at the root of the `.nupkg`, where NuGet requires it — and
/// requires exactly one.
///
/// The bare name `.nuspec` is a dotfile, not an identifier, so it does not
/// count.
pub(super) fn is_nuspec_manifest(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 1 && segs[0].len() > ".nuspec".len() && segs[0].ends_with(".nuspec")
}

/// `{distribution}-{version}.dist-info/METADATA` — PEP 427 allows a wheel
/// exactly one `.dist-info` directory.
pub(super) fn is_wheel_metadata(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 2 && segs[0].ends_with(".dist-info") && segs[1] == "METADATA"
}

/// `{name}-{version}/PKG-INFO` — an sdist's own metadata, at the wrapper
/// directory's depth.
pub(super) fn is_sdist_pkg_info(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 2 && segs[1] == "PKG-INFO"
}

/// `{name}-{version}/METADATA`, the spelling a few backends write instead.
///
/// Kept a *fallback* rather than folded into [`is_sdist_pkg_info`]: an sdist
/// carrying both would otherwise match twice and [`sole`] would call it
/// ambiguous, when in fact `PKG-INFO` is unambiguously the one to read.
pub(super) fn is_sdist_metadata_alias(path: &str) -> bool {
    let segs = segments(path);
    segs.len() == 2 && segs[1] == "METADATA"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sole_is_none_for_zero_and_for_more_than_one() {
        assert_eq!(sole(Vec::<u8>::new()), None);
        assert_eq!(sole(vec![7]), Some(7));
        assert_eq!(sole(vec![7, 8]), None);
        assert_eq!(sole(vec![7, 8, 9]), None);
    }

    #[test]
    fn empty_segments_do_not_change_depth() {
        assert!(is_npm_manifest("package//package.json"));
        assert!(is_npm_manifest("package/package.json/"));
        assert!(!is_npm_manifest("package/nested/package.json"));
    }

    #[test]
    fn the_crate_manifest_is_the_one_under_the_wrapper_directory() {
        assert!(is_crate_manifest("evil-1.0.0/Cargo.toml"));
        assert!(!is_crate_manifest("Cargo.toml"));
        assert!(!is_crate_manifest("evil-1.0.0/vendor/Cargo.toml"));
        assert!(!is_crate_manifest("evil-1.0.0/Cargo.toml.bak"));
    }

    /// The substring match that made `decoypom.xml` a manifest.
    #[test]
    fn a_file_merely_ending_in_pom_xml_is_not_a_pom() {
        assert!(is_maven_manifest("META-INF/maven/com.example/app/pom.xml"));
        assert!(!is_maven_manifest("a/decoypom.xml"));
        assert!(!is_maven_manifest("a/pom.xml"));
        assert!(!is_maven_manifest("META-INF/maven/com.example/pom.xml"));
        assert!(!is_maven_manifest(
            "META-INF/maven/com.example/app/nested/pom.xml"
        ));
    }

    #[test]
    fn the_nuspec_is_a_named_file_at_the_root() {
        assert!(is_nuspec_manifest("mylib.nuspec"));
        assert!(!is_nuspec_manifest("lib/mylib.nuspec"));
        assert!(!is_nuspec_manifest(".nuspec"));
    }

    #[test]
    fn wheel_and_sdist_metadata_sit_one_level_down() {
        assert!(is_wheel_metadata("requests-2.31.0.dist-info/METADATA"));
        assert!(!is_wheel_metadata("requests-2.31.0.dist-info/RECORD"));
        assert!(!is_wheel_metadata("nested/x.dist-info/METADATA"));

        assert!(is_sdist_pkg_info("requests-2.31.0/PKG-INFO"));
        assert!(!is_sdist_pkg_info("requests-2.31.0/METADATA"));
        assert!(!is_sdist_pkg_info("requests-2.31.0/src/PKG-INFO"));
        assert!(!is_sdist_pkg_info("PKG-INFO"));

        assert!(is_sdist_metadata_alias("requests-2.31.0/METADATA"));
        assert!(!is_sdist_metadata_alias("requests-2.31.0/vendor/METADATA"));
    }
}
