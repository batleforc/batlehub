//! One order for version strings, used everywhere the server states one.
//!
//! There were two, and they disagreed: the explore handlers sort the version
//! table with this, and `proxy::discovery::capped` *truncates* an upstream
//! version list with the order it is given. A list ordered one way and cut the
//! other way loses the rows the first order calls newest, so the two have to be
//! the same function rather than the same intention.

use std::cmp::Ordering;

/// Order two version strings newest-first.
///
/// A `String` comparison is not a version comparison, and the difference is not
/// cosmetic: `"5.6.2" > "5.10.0"` lexicographically, so `chalk` at 5.10.0 sorts
/// *below* 5.6.2 and `@types/node` 9.x sorts above 20.x. That was tolerable
/// while the console received every row and re-sorted its own view; it stopped
/// being tolerable when this order started deciding which versions page one
/// *is*, which one the server names as `default_version`, and which ones
/// `capped` throws away.
///
/// Semver where both sides parse, lexicographic where either does not.
///
/// Two normalisations before the parse, because strict semver would otherwise
/// reject most of what the registries this proxies actually publish:
///
/// - a leading `v` is dropped (Go, Terraform and several others write one);
/// - a numeric core of one or two components is padded to three, so PyPI's
///   `2.10`, RubyGems' `1.2` and Maven's `1.0-SNAPSHOT` are read as versions
///   rather than falling through to the string compare. Without it a *mixed*
///   list came out worse than plain lexicographic order: `["1.1.1", "1.2"]`
///   put `1.1.1` first, because an unparseable version sorts below every
///   parseable one, and `default_version` then named an older release.
///
/// - a numeric core of **four or more** components — NuGet's dominant scheme,
///   and every `AssemblyVersion`-shaped release — keeps its first three as the
///   semver core and the rest as a numeric tie-break. Without it those versions
///   were unparseable and fell to the reversed *string* compare below, which
///   reads `1.0.0.10` as older than `1.0.0.9`; `capped` sorts with this same
///   comparator before it truncates, so on a package with more releases than
///   `max_versions` every `*.10`–`*.99` build was the row thrown away.
///
/// What still does not parse — PyPI's `1.0rc1`, anything with a letter in the
/// core — sorts below what does, ordered among itself by a reversed string
/// compare. Arbitrary but total and stable, which is what a fallback owes the
/// caller.
pub fn newest_first(a: &str, b: &str) -> Ordering {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => b.cmp(&a),
        // A version this cannot read sorts below one it can, rather than
        // interleaving by string: `1.0rc1` next to `1.0.0` is noise, and a
        // reader looking for the newest release wants the readable ones first.
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b.cmp(a),
    }
}

/// A version this order can read: a semver, plus the numeric components beyond
/// the third that semver has nowhere to put.
///
/// The extra components are compared **after** the semver, so `1.0.0.10` and
/// `1.0.0.9` differ by the tie-break alone while `1.1.0.0` still beats
/// `1.0.0.99` on the core. A version with only three components carries an
/// empty tail, which sorts below any non-empty one — `1.0.0.1` is a later build
/// of `1.0.0`, which is how NuGet reads it too.
type Parsed = (semver::Version, Vec<u64>);

/// `semver::Version`, after the normalisations [`newest_first`] documents.
fn parse(raw: &str) -> Option<Parsed> {
    let v = raw.strip_prefix('v').unwrap_or(raw);
    if let Ok(parsed) = semver::Version::parse(v) {
        return Some((parsed, Vec::new()));
    }
    // Split the numeric core off the pre-release/build suffix before padding,
    // so `1.0-SNAPSHOT` becomes `1.0.0-SNAPSHOT` and not `1.0-SNAPSHOT.0`.
    let end = v.find(['-', '+']).unwrap_or(v.len());
    let (core, suffix) = v.split_at(end);
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    // Four or more components: the first three are the semver core, the rest is
    // the tie-break. A component too large for a `u64` is not a version number
    // anybody published — treat the whole string as unreadable rather than
    // silently comparing a saturated value.
    let mut extra = Vec::new();
    for part in parts.iter().skip(3) {
        extra.push(part.parse::<u64>().ok()?);
    }
    let mut padded: String = parts.iter().take(3).copied().collect::<Vec<_>>().join(".");
    for _ in parts.len().min(3)..3 {
        padded.push_str(".0");
    }
    padded.push_str(suffix);
    semver::Version::parse(&padded).ok().map(|v| (v, extra))
}

#[cfg(test)]
mod tests {
    use super::newest_first;
    use std::cmp::Ordering;

    /// The case a lexicographic sort gets backwards, and the reason this exists.
    #[test]
    fn a_double_digit_patch_is_newer_than_a_single_digit_one() {
        assert_eq!(newest_first("5.10.0", "5.6.2"), Ordering::Less);
        assert_eq!(newest_first("5.6.2", "5.10.0"), Ordering::Greater);
        assert_eq!(newest_first("20.1.0", "9.0.0"), Ordering::Less);
    }

    /// Pre-release ordering is semver's, not the string's: `1.0.0` is newer than
    /// `1.0.0-rc.1`, which a string compare reads the other way.
    #[test]
    fn a_release_is_newer_than_its_own_candidate() {
        assert_eq!(newest_first("1.0.0", "1.0.0-rc.1"), Ordering::Less);
    }

    #[test]
    fn a_leading_v_is_not_part_of_the_number() {
        assert_eq!(newest_first("v1.10.0", "v1.9.0"), Ordering::Less);
    }

    /// A short numeric core is a version, not an unparseable string. PyPI,
    /// RubyGems and Maven all publish two-component versions, and treating them
    /// as unreadable put them *below* every three-component one — so
    /// `["1.1.1", "1.2"]` led with `1.1.1` and `default_version` named it.
    #[test]
    fn a_two_component_version_is_read_as_one() {
        assert_eq!(newest_first("1.2", "1.1.1"), Ordering::Less);
        assert_eq!(newest_first("2.10", "2.9"), Ordering::Less);
        assert_eq!(newest_first("2", "1.9.9"), Ordering::Less);
        assert_eq!(newest_first("1.0", "1.0.0"), Ordering::Equal);
    }

    /// Nothing panics or ties on a version semver cannot read; unreadable ones
    /// sort below readable ones and are ordered among themselves.
    #[test]
    fn unparseable_versions_are_ordered_and_come_last() {
        assert_eq!(newest_first("1.0.0", "1.0.0rc1"), Ordering::Less);
        assert_eq!(newest_first("1.0.0rc1", "1.0.0"), Ordering::Greater);
        assert_eq!(
            newest_first("1.0-SNAPSHOT", "1.0-SNAPSHOT"),
            Ordering::Equal
        );
        assert_eq!(newest_first("2026.1.3", "2025.1.3"), Ordering::Less);
    }

    /// NuGet's four-part number is a version, not an unreadable string. It was
    /// falling through to the reversed *string* compare, which reads
    /// `1.0.0.10` as older than `1.0.0.9` — and `discovery::capped` truncates
    /// with this comparator, so on a package with more releases than
    /// `max_versions` every `*.10`–`*.99` build was the row thrown away.
    #[test]
    fn a_four_component_version_is_read_as_one() {
        assert_eq!(newest_first("1.0.0.10", "1.0.0.9"), Ordering::Less);
        assert_eq!(newest_first("1.0.0.9", "1.0.0.10"), Ordering::Greater);
        // The first three components still decide when they differ.
        assert_eq!(newest_first("1.1.0.0", "1.0.0.99"), Ordering::Less);
        // A fourth component makes it a later build of the same release, which
        // is how NuGet reads it too.
        assert_eq!(newest_first("1.0.0.1", "1.0.0"), Ordering::Less);
        // …and it still outranks anything genuinely unreadable.
        assert_eq!(newest_first("14.0.1.0", "13.0.3rc1"), Ordering::Less);
        // Five components, and the tie-break keeps going.
        assert_eq!(newest_first("1.0.0.0.2", "1.0.0.0.10"), Ordering::Greater);
    }

    /// Maven's `-SNAPSHOT` is a pre-release of the release it names, and after
    /// padding semver says so.
    #[test]
    fn a_snapshot_is_older_than_the_release_it_names() {
        assert_eq!(newest_first("1.0.0", "1.0-SNAPSHOT"), Ordering::Less);
        assert_eq!(newest_first("1.0-SNAPSHOT", "1.0.0"), Ordering::Greater);
    }
}
