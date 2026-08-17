//! PyPI: the simple index, in both of its representations.
//!
//! One route, two documents, chosen by `Accept`: PEP 503 HTML and PEP 691 JSON.
//! Neither names a preferred version — pip picks from the list against its own
//! constraint — so filtering is removal with nothing to repair.
//!
//! The awkward part is that a simple index lists **files**, not versions. The
//! version has to be recovered from each distribution filename, and a filename
//! this proxy cannot parse is **kept**: a naming convention it has not seen
//! should degrade to over-listing one file, never to hiding a package's entire
//! file set.

use serde_json::Value;

use super::BlockedVersions;

/// The version a PyPI distribution filename encodes, if it can be recovered.
///
/// Wheels are `{name}-{version}-{python}-{abi}-{platform}.whl` and sdists are
/// `{name}-{version}.{tar.gz,zip,tar.bz2}`; in both, the version is the first
/// `-`-separated segment after the name that starts with a digit. The name
/// itself may contain `-`, which is why this scans forward rather than taking
/// the second field.
///
/// `None` for anything unrecognised, which the callers treat as "keep".
pub fn version_from_filename(filename: &str) -> Option<&str> {
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

/// Remove blocked versions from a PEP 691 JSON simple page.
///
/// `files` is the list pip resolves against. `versions` (PEP 700) is an
/// optional summary of the same set and is filtered alongside it, or the two
/// contradict each other.
pub fn strip_simple_json(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let mut removed = Vec::new();

    if let Some(files) = doc.get_mut("files").and_then(Value::as_array_mut) {
        files.retain(|f| {
            let Some(name) = f.get("filename").and_then(Value::as_str) else {
                return true;
            };
            let Some(v) = version_from_filename(name) else {
                return true;
            };
            if blocked.contains(v) {
                removed.push(v.to_owned());
                false
            } else {
                true
            }
        });
    }

    if let Some(versions) = doc.get_mut("versions").and_then(Value::as_array_mut) {
        versions.retain(|v| !v.as_str().is_some_and(|s| blocked.contains(s)));
    }

    removed
}

/// Remove blocked versions from a PEP 503 HTML simple page.
///
/// Line-oriented rather than a DOM rewrite: a simple page is one anchor per
/// line by convention and by every generator in use, and lines with no anchor
/// (the doctype, `<head>`, the closing tags) pass through untouched. A line
/// carrying an anchor whose filename cannot be parsed is kept.
///
/// An anchor split across lines would therefore not be filtered. That is the
/// over-listing direction, and the JSON representation — which pip prefers
/// whenever the server offers it — has no such ambiguity.
pub fn strip_simple_html(html: &mut String, blocked: &BlockedVersions) -> Vec<String> {
    let mut removed = Vec::new();
    let mut kept: Vec<&str> = Vec::new();

    for line in html.lines() {
        match anchor_filename(line) {
            Some(name) => match version_from_filename(name) {
                Some(v) if blocked.contains(v) => removed.push(v.to_owned()),
                _ => kept.push(line),
            },
            None => kept.push(line),
        }
    }
    if removed.is_empty() {
        return removed;
    }

    let trailing_newline = html.ends_with('\n');
    let mut out = kept.join("\n");
    if trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    *html = out;
    removed
}

/// The distribution filename an `<a …>` line links to — its link *text*, which
/// PEP 503 defines as the filename.
///
/// Read from the text rather than the `href`, because the href is rewritten to
/// this proxy's own `packages/{filename}` route and may carry a `#sha256=`
/// fragment; the text is the filename verbatim either way.
fn anchor_filename(line: &str) -> Option<&str> {
    let after_open = line.find("<a ").map(|i| &line[i..])?;
    let text_start = after_open.find('>')? + 1;
    let text_end = after_open[text_start..].find("</a>")? + text_start;
    let text = after_open[text_start..text_end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Pypi,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    // ── version_from_filename ────────────────────────────────────────────────

    #[test]
    fn wheel_and_sdist_filenames_yield_their_version() {
        assert_eq!(
            version_from_filename("requests-2.28.0-py3-none-any.whl"),
            Some("2.28.0")
        );
        assert_eq!(
            version_from_filename("requests-2.28.0.tar.gz"),
            Some("2.28.0")
        );
        assert_eq!(version_from_filename("requests-2.28.0.zip"), Some("2.28.0"));
    }

    /// A hyphenated project name must not be mistaken for a version boundary.
    #[test]
    fn a_hyphenated_project_name_does_not_confuse_the_scan() {
        assert_eq!(
            version_from_filename("zope-interface-5.4.0.tar.gz"),
            Some("5.4.0")
        );
        assert_eq!(
            version_from_filename("typing-extensions-4.5.0-py3-none-any.whl"),
            Some("4.5.0")
        );
    }

    /// The rule that decides the safe failure direction: unparseable means
    /// *keep*, so a convention this proxy has not seen over-lists one file
    /// rather than hiding a package's whole file set.
    #[test]
    fn an_unrecognised_filename_has_no_version() {
        assert_eq!(version_from_filename("requests.tar.xz"), None);
        assert_eq!(version_from_filename("README"), None);
        assert_eq!(version_from_filename("requests-.whl"), None);
    }

    // ── PEP 691 JSON ─────────────────────────────────────────────────────────

    fn json_page() -> Value {
        json!({
            "name": "requests",
            "versions": ["2.27.0", "2.28.0"],
            "files": [
                { "filename": "requests-2.27.0.tar.gz", "url": "https://cdn/requests-2.27.0.tar.gz" },
                { "filename": "requests-2.28.0-py3-none-any.whl", "url": "https://cdn/x.whl" },
                { "filename": "requests-2.28.0.tar.gz", "url": "https://cdn/requests-2.28.0.tar.gz" }
            ]
        })
    }

    /// A block on a version hides *every* file of that version — the wheel and
    /// the sdist both.
    #[test]
    fn json_page_drops_every_file_of_a_blocked_version() {
        let mut doc = json_page();
        let removed = strip_simple_json(&mut doc, &blocked(&["2.28.0"]));

        assert_eq!(removed.len(), 2, "the wheel and the sdist");
        let names: Vec<&str> = doc["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["filename"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["requests-2.27.0.tar.gz"]);
    }

    /// PEP 700's `versions` summary describes the same set as `files`; leaving
    /// it alone would have the document contradict itself.
    #[test]
    fn json_page_filters_the_pep_700_versions_summary_too() {
        let mut doc = json_page();
        strip_simple_json(&mut doc, &blocked(&["2.28.0"]));

        assert_eq!(doc["versions"], json!(["2.27.0"]));
    }

    #[test]
    fn json_page_matches_across_pep440_spellings() {
        let mut doc = json!({
            "files": [{ "filename": "pkg-1.0.0.tar.gz" }]
        });
        // PEP 440 zero-pads for comparison, so `1.0` and `1.0.0` are one version.
        strip_simple_json(&mut doc, &blocked(&["1.0"]));

        assert_eq!(doc["files"], json!([]));
    }

    #[test]
    fn json_page_keeps_a_file_whose_name_it_cannot_parse() {
        let mut doc = json!({ "files": [{ "filename": "mystery.tar.xz" }] });
        let before = doc.clone();
        assert!(strip_simple_json(&mut doc, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn json_page_blocking_everything_leaves_a_well_formed_empty_page() {
        let mut doc = json_page();
        strip_simple_json(&mut doc, &blocked(&["2.27.0", "2.28.0"]));

        assert_eq!(doc["files"], json!([]));
        assert_eq!(doc["name"], json!("requests"), "the envelope survives");
    }

    // ── PEP 503 HTML ─────────────────────────────────────────────────────────

    fn html_page() -> String {
        concat!(
            "<!DOCTYPE html>\n",
            "<html><body>\n",
            "<a href=\"/proxy/p/packages/requests-2.27.0.tar.gz#sha256=aaa\">requests-2.27.0.tar.gz</a><br/>\n",
            "<a href=\"/proxy/p/packages/requests-2.28.0.tar.gz#sha256=bbb\">requests-2.28.0.tar.gz</a><br/>\n",
            "</body></html>\n"
        )
        .to_owned()
    }

    #[test]
    fn html_page_drops_the_blocked_anchor() {
        let mut html = html_page();
        let removed = strip_simple_html(&mut html, &blocked(&["2.28.0"]));

        assert_eq!(removed, vec!["2.28.0".to_owned()]);
        assert!(!html.contains("2.28.0"));
        assert!(html.contains("requests-2.27.0.tar.gz"));
        assert!(html.contains("</body></html>"), "the envelope survives");
    }

    #[test]
    fn html_page_blocking_an_absent_version_is_byte_identical() {
        let mut html = html_page();
        let before = html.clone();
        assert!(strip_simple_html(&mut html, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(html, before);
    }

    #[test]
    fn html_page_blocking_everything_leaves_the_document_structure() {
        let mut html = html_page();
        strip_simple_html(&mut html, &blocked(&["2.27.0", "2.28.0"]));

        assert!(!html.contains("<a "));
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</body></html>"));
    }

    #[test]
    fn html_lines_without_an_anchor_are_never_touched() {
        let mut html = "<!DOCTYPE html>\n<h1>Links for requests</h1>\n".to_owned();
        let before = html.clone();
        assert!(strip_simple_html(&mut html, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(html, before);
    }

    #[test]
    fn anchor_text_is_the_filename_not_the_href() {
        assert_eq!(
            anchor_filename("<a href=\"/x/y#sha256=z\">pkg-1.0.0.tar.gz</a><br/>"),
            Some("pkg-1.0.0.tar.gz")
        );
        assert_eq!(anchor_filename("<p>no anchor here</p>"), None);
    }
}
