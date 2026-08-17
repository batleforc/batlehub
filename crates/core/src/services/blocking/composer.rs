//! Composer: p2 metadata, including the minified format.
//!
//! A p2 document is `{"packages": {"vendor/name": [ …version objects… ]}}`, and
//! filtering it would be unremarkable except for one thing: Packagist serves
//! `"minified": "composer/2.0"`, in which **each entry after the first omits
//! every key identical to the previous entry**, and a key set to `null` means
//! "this key is absent from here on".
//!
//! Removing a middle entry from a minified list therefore changes what every
//! entry after it inherits — silently, and in a way that produces a
//! well-formed document describing the wrong packages. So the filter
//! **expands, filters, then re-minifies**, and the regression test for that is
//! the one test in this module that matters.
//!
//! Nothing in a p2 document names a preferred version; Composer picks from the
//! list against the constraint in `composer.json`.

use serde_json::{Map, Value};

use super::{encode_package_segment, BlockedVersions};

/// Remove blocked versions from a p2 metadata document.
///
/// Handles both the plain and the minified encodings; a minified list is
/// expanded before filtering and re-minified afterwards, so an entry that
/// inherited a key from a removed predecessor keeps the value it had.
pub fn strip_p2(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let minified = doc
        .get("minified")
        .and_then(Value::as_str)
        .is_some_and(|m| m == "composer/2.0");

    let Some(packages) = doc.get_mut("packages").and_then(Value::as_object_mut) else {
        return Vec::new();
    };

    let mut removed = Vec::new();
    for (_name, entries) in packages.iter_mut() {
        let Some(list) = entries.as_array() else {
            continue;
        };
        let expanded = if minified { expand(list) } else { list.clone() };

        let kept: Vec<Value> = expanded
            .into_iter()
            .filter(|e| match e.get("version").and_then(Value::as_str) {
                Some(v) if blocked.contains(v) => {
                    removed.push(v.to_owned());
                    false
                }
                _ => true,
            })
            .collect();

        *entries = Value::Array(if minified { minify(&kept) } else { kept });
    }

    removed
}

/// Point every `dist.url` at this proxy's own download route.
///
/// Served unrewritten, a p2 document hands Composer the upstream CDN and every
/// download routes around this proxy — past its cache, its audit trail and the
/// download-time gate that is a block's other half.
///
/// The route shape matches what the local registry already serves, and what
/// `composer_dist` parses: `{base}/dist/{vendor}/{name}/{version}` — with **no**
/// extension, since the route reads the last segment as the version and a
/// `.zip` suffix would make it look for a version literally called `1.0.0.zip`.
pub fn rewrite_dist_urls(doc: &mut Value, public_base: &str) {
    let base = public_base.trim_end_matches('/');
    let Some(packages) = doc.get_mut("packages").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, entries) in packages.iter_mut() {
        let encoded = name
            .split('/')
            .map(encode_package_segment)
            .collect::<Vec<_>>()
            .join("/");
        let Some(list) = entries.as_array_mut() else {
            continue;
        };
        for entry in list.iter_mut() {
            // In a minified list an entry that omits `version` inherits it, and
            // an entry that omits `dist` inherits that too — so only entries
            // carrying both are rewritten, and the rest keep inheriting a URL
            // this proxy already rewrote.
            let Some(version) = entry
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            let Some(dist) = entry.get_mut("dist").and_then(Value::as_object_mut) else {
                continue;
            };
            dist.insert(
                "url".to_owned(),
                Value::String(format!("{base}/dist/{encoded}/{version}")),
            );
        }
    }
}

/// Expand a `composer/2.0` minified list: every entry gets the full key set,
/// inheriting from its predecessor, with `null` meaning "removed from here on".
fn expand(list: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(list.len());
    let mut current: Map<String, Value> = Map::new();

    for entry in list {
        let Some(obj) = entry.as_object() else {
            out.push(entry.clone());
            continue;
        };
        for (k, v) in obj {
            if v.is_null() {
                current.remove(k);
            } else {
                current.insert(k.clone(), v.clone());
            }
        }
        out.push(Value::Object(current.clone()));
    }
    out
}

/// The inverse of [`expand`]: emit only what changed from the previous entry,
/// with `null` for a key that has gone away.
fn minify(list: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(list.len());
    let mut previous: Map<String, Value> = Map::new();

    for entry in list {
        let Some(obj) = entry.as_object() else {
            out.push(entry.clone());
            continue;
        };
        let mut delta = Map::new();
        for (k, v) in obj {
            if previous.get(k) != Some(v) {
                delta.insert(k.clone(), v.clone());
            }
        }
        for k in previous.keys() {
            if !obj.contains_key(k) {
                delta.insert(k.clone(), Value::Null);
            }
        }
        previous = obj.clone();
        out.push(Value::Object(delta));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Composer,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn plain() -> Value {
        json!({
            "packages": {
                "monolog/monolog": [
                    { "name": "monolog/monolog", "version": "3.0.0",
                      "dist": { "type": "zip", "url": "https://cdn.invalid/3.0.0.zip" } },
                    { "name": "monolog/monolog", "version": "2.9.0",
                      "dist": { "type": "zip", "url": "https://cdn.invalid/2.9.0.zip" } },
                    { "name": "monolog/monolog", "version": "2.8.0",
                      "dist": { "type": "zip", "url": "https://cdn.invalid/2.8.0.zip" } }
                ]
            }
        })
    }

    fn versions(doc: &Value) -> Vec<String> {
        doc["packages"]["monolog/monolog"]
            .as_array()
            .expect("a version list")
            .iter()
            .map(|e| e["version"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    #[test]
    fn plain_p2_drops_the_blocked_entry() {
        let mut doc = plain();
        let removed = strip_p2(&mut doc, &blocked(&["2.9.0"]));

        assert_eq!(removed, vec!["2.9.0".to_owned()]);
        assert_eq!(versions(&doc), ["3.0.0", "2.8.0"]);
    }

    #[test]
    fn plain_p2_blocking_an_absent_version_changes_nothing() {
        let mut doc = plain();
        let before = doc.clone();
        assert!(strip_p2(&mut doc, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn plain_p2_blocking_everything_leaves_an_empty_list() {
        let mut doc = plain();
        strip_p2(&mut doc, &blocked(&["3.0.0", "2.9.0", "2.8.0"]));

        assert_eq!(doc["packages"]["monolog/monolog"], json!([]));
    }

    // ── the minified format ──────────────────────────────────────────────────

    /// The document Packagist actually serves. `2.9.0` omits `name` and `type`
    /// (identical to `3.0.0`'s) and `2.8.0` omits everything but its version
    /// and dist — so removing `2.9.0` naively leaves `2.8.0` inheriting from
    /// `3.0.0` instead, and the entry silently changes meaning.
    fn minified() -> Value {
        json!({
            "minified": "composer/2.0",
            "packages": {
                "monolog/monolog": [
                    { "name": "monolog/monolog", "version": "3.0.0", "license": ["MIT"],
                      "require": { "php": ">=8.1" } },
                    { "version": "2.9.0", "require": { "php": ">=7.2" } },
                    { "version": "2.8.0" }
                ]
            }
        })
    }

    /// The regression that catches silent corruption: what survives must expand
    /// to exactly what it expanded to before, minus the removed version.
    #[test]
    fn removing_a_middle_entry_does_not_change_what_later_entries_inherit() {
        let before_expanded = expand(
            minified()["packages"]["monolog/monolog"]
                .as_array()
                .unwrap(),
        );
        let before_2_8_0 = before_expanded
            .iter()
            .find(|e| e["version"] == "2.8.0")
            .unwrap()
            .clone();
        assert_eq!(
            before_2_8_0["require"]["php"], ">=7.2",
            "the fixture has to actually exercise inheritance"
        );

        let mut doc = minified();
        let removed = strip_p2(&mut doc, &blocked(&["2.9.0"]));
        assert_eq!(removed, vec!["2.9.0".to_owned()]);

        let after_expanded = expand(doc["packages"]["monolog/monolog"].as_array().unwrap());
        let after_2_8_0 = after_expanded
            .iter()
            .find(|e| e["version"] == "2.8.0")
            .expect("2.8.0 survives");
        assert_eq!(
            *after_2_8_0, before_2_8_0,
            "2.8.0 inherited from the entry that was removed and now says something else"
        );
    }

    #[test]
    fn a_filtered_minified_document_is_still_minified() {
        let mut doc = minified();
        strip_p2(&mut doc, &blocked(&["2.9.0"]));

        let list = doc["packages"]["monolog/monolog"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert!(
            list[1].as_object().unwrap().len() < list[0].as_object().unwrap().len(),
            "the second entry should still omit what it inherits: {list:?}"
        );
        assert_eq!(doc["minified"], "composer/2.0");
    }

    #[test]
    fn expand_then_minify_round_trips() {
        let list = minified()["packages"]["monolog/monolog"]
            .as_array()
            .unwrap()
            .clone();
        let round_tripped = minify(&expand(&list));

        assert_eq!(expand(&round_tripped), expand(&list));
    }

    /// A key that disappears mid-list is `null` in the minified encoding.
    #[test]
    fn minify_marks_a_dropped_key_as_null() {
        let expanded = vec![
            json!({ "version": "2.0.0", "abandoned": true }),
            json!({ "version": "3.0.0" }),
        ];
        let out = minify(&expanded);

        assert_eq!(out[1]["abandoned"], Value::Null);
        assert_eq!(expand(&out), expanded);
    }

    // ── URL rewriting ────────────────────────────────────────────────────────

    #[test]
    fn dist_urls_point_back_at_this_proxy() {
        let mut doc = plain();
        rewrite_dist_urls(&mut doc, "https://hub.example.com/proxy/php1/");

        let list = doc["packages"]["monolog/monolog"].as_array().unwrap();
        assert_eq!(
            list[0]["dist"]["url"], "https://hub.example.com/proxy/php1/dist/monolog/monolog/3.0.0",
            "no extension: `composer_dist` reads the last segment as the version"
        );
        assert_eq!(
            list[0]["dist"]["type"], "zip",
            "the rest of the dist block survives"
        );
    }

    #[test]
    fn an_entry_with_no_dist_block_is_left_alone() {
        let mut doc = json!({ "packages": { "a/b": [{ "version": "1.0.0" }] } });
        let before = doc.clone();
        rewrite_dist_urls(&mut doc, "https://h/proxy/p");
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_document_is_returned_unchanged() {
        let mut doc = json!({ "not-packages": {} });
        let before = doc.clone();
        assert!(strip_p2(&mut doc, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }
}
