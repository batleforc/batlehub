//! Go module proxy: `@v/list` and `@latest`.
//!
//! `@v/list` is newline-delimited text, one version per line — the only
//! protocol here whose listing is not a structured document. `@latest` is JSON
//! naming exactly one version and carrying no list at all, which is why
//! repairing it is a composition rather than a rewrite: see
//! [`repaired_latest`].

use serde_json::Value;

use super::{best_latest, BlockedVersions};

/// Remove blocked versions from a `@v/list` body.
///
/// Line order is preserved. Blank lines and anything that is not a bare version
/// are kept: `@v/list` is defined as one version per line, and a body that
/// disagrees is one this proxy does not understand well enough to edit.
pub fn strip_version_list(body: &mut String, blocked: &BlockedVersions) -> Vec<String> {
    let mut removed = Vec::new();
    let mut kept: Vec<&str> = Vec::new();

    for line in body.lines() {
        let v = line.trim();
        if !v.is_empty() && blocked.contains(v) {
            removed.push(v.to_owned());
        } else {
            kept.push(line);
        }
    }
    if removed.is_empty() {
        return removed;
    }

    let mut out = kept.join("\n");
    // `go` tolerates a missing final newline, but every real proxy sends one and
    // an emptied list should be an empty body rather than a stray newline.
    if !out.is_empty() {
        out.push('\n');
    }
    *body = out;
    removed
}

/// Every version named by a (already filtered) `@v/list` body.
pub fn versions_in_list(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The version an `@latest` document names.
pub fn latest_version(doc: &Value) -> Option<&str> {
    doc.get("Version").and_then(Value::as_str)
}

/// Repair an `@latest` document against the versions that survived filtering.
///
/// `@latest` names one version and carries no list, so filtering it is
/// re-resolution rather than removal: the caller supplies the *filtered*
/// `@v/list`, and this picks the highest surviving version when the document
/// names a blocked one.
///
/// Returns `None` when nothing survives — the caller should `404`, which is
/// what the Go client already handles for a module with no releases.
///
/// The rebuilt document carries `Version` and drops `Time`. `Time` belongs to
/// the release `@latest` originally named, and that release is precisely the
/// one being hidden; the proxy protocol makes `Time` optional, so omitting it
/// is honest where copying the blocked version's timestamp onto a different
/// release would not be.
pub fn repaired_latest(doc: &Value, allowed: &[String]) -> Option<Value> {
    let named = latest_version(doc);
    if named.is_some_and(|v| allowed.iter().any(|a| a == v)) {
        return Some(doc.clone());
    }
    let best = best_latest(allowed)?;
    Some(serde_json::json!({ "Version": best }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Goproxy,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    #[test]
    fn version_list_drops_the_blocked_line_and_keeps_order() {
        let mut body = "v1.0.0\nv1.1.0\nv1.2.0\n".to_owned();
        let removed = strip_version_list(&mut body, &blocked(&["v1.1.0"]));

        assert_eq!(removed, vec!["v1.1.0".to_owned()]);
        assert_eq!(body, "v1.0.0\nv1.2.0\n");
    }

    /// Go's `v` prefix and `+incompatible` suffix name the same release, so a
    /// block recorded either way must hide the listed spelling.
    #[test]
    fn version_list_matches_across_go_version_spellings() {
        let mut body = "v1.0.0\nv2.0.0+incompatible\n".to_owned();
        strip_version_list(&mut body, &blocked(&["2.0.0"]));

        assert_eq!(body, "v1.0.0\n");
    }

    #[test]
    fn blocking_every_version_leaves_an_empty_body() {
        let mut body = "v1.0.0\nv1.1.0\n".to_owned();
        strip_version_list(&mut body, &blocked(&["v1.0.0", "v1.1.0"]));

        assert_eq!(body, "", "not a lone newline");
    }

    #[test]
    fn blocking_an_absent_version_leaves_the_body_byte_identical() {
        let mut body = "v1.0.0\nv1.1.0".to_owned();
        let before = body.clone();
        assert!(strip_version_list(&mut body, &blocked(&["v9.9.9"])).is_empty());
        assert_eq!(body, before, "including its missing trailing newline");
    }

    #[test]
    fn latest_naming_an_allowed_version_is_left_alone() {
        let doc = json!({ "Version": "v1.1.0", "Time": "2024-01-01T00:00:00Z" });
        let allowed = vec!["v1.0.0".to_owned(), "v1.1.0".to_owned()];

        assert_eq!(repaired_latest(&doc, &allowed), Some(doc.clone()));
    }

    #[test]
    fn latest_naming_a_blocked_version_re_resolves_to_the_best_survivor() {
        let doc = json!({ "Version": "v1.2.0", "Time": "2024-06-01T00:00:00Z" });
        let allowed = vec!["v1.0.0".to_owned(), "v1.1.0".to_owned()];

        let repaired = repaired_latest(&doc, &allowed).unwrap();
        assert_eq!(repaired["Version"], json!("v1.1.0"));
        assert!(
            repaired.get("Time").is_none(),
            "the timestamp belonged to the version being hidden"
        );
    }

    #[test]
    fn latest_with_nothing_surviving_is_a_not_found() {
        let doc = json!({ "Version": "v1.2.0" });
        assert_eq!(repaired_latest(&doc, &[]), None);
    }

    #[test]
    fn versions_in_list_ignores_blank_lines() {
        assert_eq!(
            versions_in_list("v1.0.0\n\n  v1.1.0  \n"),
            vec!["v1.0.0".to_owned(), "v1.1.0".to_owned()]
        );
    }
}
