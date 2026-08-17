//! cargo: the sparse index.
//!
//! The one protocol here where a blocked version is **marked rather than
//! removed**. Each line of the sparse index is a JSON object with `vers` and
//! `yanked`, and `yanked` is cargo's own word for "this exists, do not select
//! it": resolution skips a yanked version, while an existing `Cargo.lock` that
//! already pins it still resolves — and then meets the download gate, which is
//! the right place for that conversation.
//!
//! Deleting the line instead makes cargo report the crate as never having had
//! that version, which breaks lockfile diagnostics for no gain: the developer
//! gets "no matching package named `x` found" instead of the operator's reason.

use serde_json::Value;

use super::BlockedVersions;

/// Mark every blocked version in a sparse-index body as `yanked`.
///
/// Line order and every other field are preserved — cargo reads the index as a
/// stream and a reordered file is a different resolution. A line that does not
/// parse as JSON is passed through byte-for-byte.
pub fn mark_yanked(body: &mut String, blocked: &BlockedVersions) -> Vec<String> {
    let mut marked = Vec::new();
    let mut out = String::with_capacity(body.len());

    for line in body.lines() {
        let rewritten = mark_line(line, blocked).inspect(|(_, v)| marked.push(v.clone()));
        match rewritten {
            Some((json, _)) => out.push_str(&json),
            None => out.push_str(line),
        }
        out.push('\n');
    }

    if marked.is_empty() {
        return marked;
    }
    // The trailing newline is normalised to "one per line". A sparse index
    // without a final newline is legal and cargo reads it either way, so this
    // only shows up as a byte difference on an index that was already filtered.
    if !body.ends_with('\n') {
        out.pop();
    }
    *body = out;
    marked
}

/// `Some((rewritten_line, version))` when this line names a blocked version
/// that was not already yanked.
fn mark_line(line: &str, blocked: &BlockedVersions) -> Option<(String, String)> {
    if line.trim().is_empty() {
        return None;
    }
    let mut value: Value = serde_json::from_str(line).ok()?;
    let version = value.get("vers")?.as_str()?.to_owned();
    if !blocked.contains(&version) {
        return None;
    }
    // Already yanked upstream: nothing to change, and reporting it as newly
    // hidden would inflate the "versions hidden" counter with no-ops.
    if value.get("yanked").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    value
        .as_object_mut()?
        .insert("yanked".to_owned(), true.into());
    Some((serde_json::to_string(&value).ok()?, version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Cargo,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn index() -> String {
        concat!(
            r#"{"name":"serde","vers":"1.0.0","deps":[],"cksum":"aaa","yanked":false}"#,
            "\n",
            r#"{"name":"serde","vers":"1.1.0","deps":[],"cksum":"bbb","yanked":false}"#,
            "\n",
        )
        .to_owned()
    }

    fn line(body: &str, n: usize) -> Value {
        serde_json::from_str(body.lines().nth(n).unwrap()).unwrap()
    }

    /// The behaviour that separates cargo from every other protocol here: the
    /// version is still listed, and flagged.
    #[test]
    fn a_blocked_version_is_marked_yanked_not_removed() {
        let mut body = index();
        let marked = mark_yanked(&mut body, &blocked(&["1.1.0"]));

        assert_eq!(marked, vec!["1.1.0".to_owned()]);
        assert_eq!(body.lines().count(), 2, "the line is still there");
        assert_eq!(line(&body, 1)["yanked"], Value::Bool(true));
        assert_eq!(line(&body, 0)["yanked"], Value::Bool(false));
    }

    #[test]
    fn every_other_field_of_a_marked_line_survives() {
        let mut body = index();
        mark_yanked(&mut body, &blocked(&["1.1.0"]));

        let l = line(&body, 1);
        assert_eq!(l["name"], "serde");
        assert_eq!(l["vers"], "1.1.0");
        assert_eq!(l["cksum"], "bbb", "cargo verifies against this");
        assert!(l["deps"].is_array());
    }

    #[test]
    fn line_order_is_preserved() {
        let mut body = index();
        mark_yanked(&mut body, &blocked(&["1.0.0"]));

        assert_eq!(line(&body, 0)["vers"], "1.0.0");
        assert_eq!(line(&body, 1)["vers"], "1.1.0");
    }

    #[test]
    fn blocking_an_absent_version_leaves_the_body_byte_identical() {
        let mut body = index();
        let before = body.clone();
        assert!(mark_yanked(&mut body, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(body, before);
    }

    /// A version upstream already yanked is not reported as newly hidden — the
    /// counter would otherwise tick on every request for an unchanged document.
    #[test]
    fn an_already_yanked_version_is_not_reported_again() {
        let mut body = "{\"name\":\"serde\",\"vers\":\"1.0.0\",\"yanked\":true}\n".to_owned();
        let before = body.clone();
        assert!(mark_yanked(&mut body, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(body, before);
    }

    #[test]
    fn a_line_that_is_not_json_is_passed_through_untouched() {
        let mut body = format!("not json at all\n{}", index());
        let marked = mark_yanked(&mut body, &blocked(&["1.1.0"]));

        assert_eq!(marked, vec!["1.1.0".to_owned()]);
        assert_eq!(body.lines().next(), Some("not json at all"));
    }

    #[test]
    fn blocking_every_version_marks_every_line() {
        let mut body = index();
        mark_yanked(&mut body, &blocked(&["1.0.0", "1.1.0"]));

        assert!(body
            .lines()
            .all(|l| serde_json::from_str::<Value>(l).unwrap()["yanked"] == Value::Bool(true)));
    }
}
