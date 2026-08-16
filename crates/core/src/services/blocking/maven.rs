//! Maven: `maven-metadata.xml`.
//!
//! The only listing this proxy has to *rewrite* rather than read, and the only
//! one in XML. Two things make it more than a search-and-delete:
//!
//! - `<latest>` and `<release>` name preferred versions and come **before**
//!   `<versions>` in the document, so the rewrite is two passes: work out what
//!   survives, then stream the document out with the survivors' answers
//!   substituted.
//! - `<release>` skips qualified versions the way Maven's own resolution does —
//!   `1.0-SNAPSHOT` is a candidate for `<latest>` and never for `<release>`.
//!
//! Parsed with `quick-xml` rather than the tag-slicing the Maven adapter uses
//! for reads. A read that mis-slices returns a wrong string; a *rewrite* that
//! mis-slices emits a document Maven cannot parse at all. quick-xml also never
//! resolves external entities, so a hostile upstream gets no XXE primitive.

use quick_xml::events::{BytesText, Event};
use quick_xml::{Reader, Writer};

use super::BlockedVersions;

/// Remove blocked versions from a `maven-metadata.xml` body and repair
/// `<latest>` and `<release>`.
///
/// A document that does not parse is **left exactly as it was** and the caller
/// warned: over-listing is the safe direction, and half-rewritten XML is worse
/// than unfiltered XML.
pub fn strip_metadata(xml: &mut String, blocked: &BlockedVersions) -> Vec<String> {
    let listed = versions_in(xml);
    let removed: Vec<String> = listed
        .iter()
        .filter(|v| blocked.contains(v))
        .cloned()
        .collect();
    if removed.is_empty() {
        return removed;
    }

    let surviving: Vec<String> = listed
        .into_iter()
        .filter(|v| !blocked.contains(v))
        .collect();
    // `<latest>` is the newest *anything*, snapshots included — deliberately not
    // `best_latest`, which prefers a stable release over a newer pre-release
    // because that is what npm's `dist-tags.latest` means. Maven draws the same
    // distinction with two elements instead of one, so `<release>` is where the
    // preference for a stable version lives.
    let latest = highest(&surviving);
    let release = highest(
        &surviving
            .iter()
            .filter(|v| !is_qualified(v))
            .cloned()
            .collect::<Vec<_>>(),
    );

    match rewrite(xml, blocked, latest.as_deref(), release.as_deref()) {
        Some(out) => {
            *xml = out;
            removed
        }
        None => {
            tracing::warn!(
                "maven-metadata.xml did not parse; serving it unfiltered rather than \
                 half-rewritten"
            );
            Vec::new()
        }
    }
}

/// Every `<version>` in the document, in document order.
fn versions_in(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut in_version = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == b"version" => in_version = true,
            Ok(Event::End(e)) if e.name().as_ref() == b"version" => in_version = false,
            Ok(Event::Text(t)) if in_version => {
                if let Ok(s) = t.decode() {
                    let s = s.trim();
                    if !s.is_empty() {
                        out.push(s.to_owned());
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Stream the document out, dropping blocked `<version>` elements and
/// substituting the recomputed `<latest>`/`<release>` text.
///
/// `None` on a parse error, which the caller turns into "serve it unchanged".
fn rewrite(
    xml: &str,
    blocked: &BlockedVersions,
    latest: Option<&str>,
    release: Option<&str>,
) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::new());

    // `<version>` is buffered because whether to emit it is only known once its
    // text has been read; `substituting` holds the replacement text for the
    // single-valued element currently open.
    let mut pending_version: Option<Vec<Event<'_>>> = None;
    let mut substituting: Option<Option<String>> = None;

    loop {
        let event = match reader.read_event() {
            Ok(e) => e,
            Err(_) => return None,
        };

        match &event {
            Event::Eof => break,

            Event::Start(e) if e.name().as_ref() == b"version" => {
                pending_version = Some(vec![Event::Start(e.to_owned())]);
            }
            Event::Start(e) if e.name().as_ref() == b"latest" => {
                substituting = Some(latest.map(str::to_owned));
                writer.write_event(Event::Start(e.to_owned())).ok()?;
            }
            Event::Start(e) if e.name().as_ref() == b"release" => {
                substituting = Some(release.map(str::to_owned));
                writer.write_event(Event::Start(e.to_owned())).ok()?;
            }

            Event::End(e) if e.name().as_ref() == b"version" => {
                let mut buffered = pending_version.take().unwrap_or_default();
                buffered.push(Event::End(e.to_owned()));
                let text = buffered
                    .iter()
                    .find_map(|ev| match ev {
                        Event::Text(t) => t.decode().ok().map(|s| s.trim().to_owned()),
                        _ => None,
                    })
                    .unwrap_or_default();
                if !blocked.contains(&text) {
                    for ev in buffered {
                        writer.write_event(ev).ok()?;
                    }
                }
                // A dropped `<version>` takes its surrounding whitespace with it,
                // because the indentation text node belonging to the *next*
                // element is written normally. Emitting it would leave a blank
                // line where the entry used to be.
            }
            Event::End(e) if matches!(e.name().as_ref(), b"latest" | b"release") => {
                substituting = None;
                writer.write_event(Event::End(e.to_owned())).ok()?;
            }

            Event::Text(t) => {
                if let Some(buffered) = pending_version.as_mut() {
                    buffered.push(Event::Text(t.to_owned()));
                } else if let Some(replacement) = substituting.as_ref() {
                    // `None` means nothing survives to name. The element is left
                    // empty rather than removed: Maven reads an absent
                    // `<release>` and an empty one the same way, and removing an
                    // element mid-stream would need its end tag suppressed too.
                    writer
                        .write_event(Event::Text(BytesText::new(
                            replacement.as_deref().unwrap_or(""),
                        )))
                        .ok()?;
                } else {
                    writer.write_event(Event::Text(t.to_owned())).ok()?;
                }
            }

            other => {
                if let Some(buffered) = pending_version.as_mut() {
                    buffered.push(other.to_owned());
                } else {
                    writer.write_event(other.to_owned()).ok()?;
                }
            }
        }
    }

    String::from_utf8(writer.into_inner()).ok()
}

/// The highest of `versions`, with no preference for stable over qualified.
///
/// Semver ordering where the strings parse — which puts `2.0.0-SNAPSHOT` above
/// `1.0.0`, as Maven does — and lexicographic where they do not, since Maven
/// version strings are not required to be semver.
fn highest(versions: &[String]) -> Option<String> {
    let mut best: Option<(&String, Option<semver::Version>)> = None;
    for v in versions {
        let parsed = semver::Version::parse(v).ok();
        let wins = match (&best, &parsed) {
            (None, _) => true,
            (Some((_, Some(b))), Some(p)) => p > b,
            // A parseable version beats an unparseable one; between two
            // unparseable ones, lexicographic order is the only tiebreak there is.
            (Some((_, None)), Some(_)) => true,
            (Some((_, Some(_))), None) => false,
            (Some((b, None)), None) => v > *b,
        };
        if wins {
            best = Some((v, parsed));
        }
    }
    best.map(|(v, _)| v.clone())
}

/// Maven's `<release>` names the newest *released* version, which excludes
/// snapshots. Anything carrying a qualifier after the numeric core is a
/// candidate for `<latest>` only.
fn is_qualified(version: &str) -> bool {
    version.contains("SNAPSHOT") || version.contains('-') || version.contains("alpha")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Maven,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn metadata() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>mylib</artifactId>
  <versioning>
    <latest>2.0.0-SNAPSHOT</latest>
    <release>1.2.0</release>
    <versions>
      <version>1.0.0</version>
      <version>1.2.0</version>
      <version>2.0.0-SNAPSHOT</version>
    </versions>
    <lastUpdated>20240315143022</lastUpdated>
  </versioning>
</metadata>"#
            .to_owned()
    }

    #[test]
    fn a_blocked_version_leaves_the_versions_list() {
        let mut xml = metadata();
        let removed = strip_metadata(&mut xml, &blocked(&["1.2.0"]));

        assert_eq!(removed, vec!["1.2.0".to_owned()]);
        assert!(!xml.contains("<version>1.2.0</version>"));
        assert!(xml.contains("<version>1.0.0</version>"));
        assert!(xml.contains("<version>2.0.0-SNAPSHOT</version>"));
    }

    /// `<release>` must fall back to the newest surviving *non-snapshot*
    /// version, not to the newer snapshot — that is the distinction the two
    /// elements exist to draw.
    #[test]
    fn release_falls_back_past_snapshots_and_latest_does_not() {
        let mut xml = metadata();
        strip_metadata(&mut xml, &blocked(&["1.2.0"]));

        assert!(
            xml.contains("<release>1.0.0</release>"),
            "release should skip the snapshot: {xml}"
        );
        assert!(
            xml.contains("<latest>2.0.0-SNAPSHOT</latest>"),
            "latest may name a snapshot: {xml}"
        );
    }

    #[test]
    fn blocking_the_newest_release_moves_both_pointers() {
        let mut xml = metadata();
        strip_metadata(&mut xml, &blocked(&["1.2.0", "2.0.0-SNAPSHOT"]));

        assert!(xml.contains("<latest>1.0.0</latest>"), "{xml}");
        assert!(xml.contains("<release>1.0.0</release>"), "{xml}");
    }

    #[test]
    fn blocking_every_version_leaves_a_well_formed_empty_document() {
        let mut xml = metadata();
        strip_metadata(&mut xml, &blocked(&["1.0.0", "1.2.0", "2.0.0-SNAPSHOT"]));

        assert!(!xml.contains("<version>"), "{xml}");
        assert!(xml.contains("<versions>"), "the element itself survives");
        assert!(xml.contains("<latest></latest>"), "{xml}");
        assert!(
            xml.contains("<artifactId>mylib</artifactId>"),
            "the envelope survives"
        );
    }

    #[test]
    fn blocking_an_absent_version_leaves_the_document_byte_identical() {
        let mut xml = metadata();
        let before = xml.clone();
        assert!(strip_metadata(&mut xml, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(xml, before, "including whitespace and the XML declaration");
    }

    /// Half-rewritten XML is worse than unfiltered XML: Maven rejects the
    /// former outright and merely over-lists on the latter.
    #[test]
    fn a_malformed_document_is_returned_unchanged() {
        let mut xml = "<metadata><versions><version>1.0.0</versions>".to_owned();
        let before = xml.clone();
        let removed = strip_metadata(&mut xml, &blocked(&["1.0.0"]));

        assert!(removed.is_empty());
        assert_eq!(xml, before);
    }

    #[test]
    fn the_group_and_artifact_ids_survive_a_filter() {
        let mut xml = metadata();
        strip_metadata(&mut xml, &blocked(&["1.0.0"]));

        assert!(xml.contains("<groupId>com.example</groupId>"));
        assert!(xml.contains("<lastUpdated>20240315143022</lastUpdated>"));
    }

    #[test]
    fn snapshots_and_qualified_versions_are_not_releases() {
        assert!(is_qualified("2.0.0-SNAPSHOT"));
        assert!(is_qualified("1.0-alpha"));
        assert!(is_qualified("1.0.0-rc1"));
        assert!(!is_qualified("1.2.0"));
    }
}
