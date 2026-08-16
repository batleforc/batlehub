//! conda: `repodata.json` and `current_repodata.json`.
//!
//! The only listing here that describes **many packages at once**, which
//! changes two things.
//!
//! The blocked set is a whole registry's worth rather than one package's, so it
//! comes from [`crate::ports::PackageRepository::blocked_in_registry`] behind a
//! short-lived snapshot: `repodata.json` for a busy channel is tens of
//! megabytes and is fetched on every `conda install`, and re-querying per
//! request would put the channel's entire block list on that path.
//!
//! And entries are keyed by *filename* — `numpy-1.24.0-py311_0.conda` — with
//! the name and version carried inside the entry rather than parsed out of the
//! key, which is a mercy after PyPI.

use serde_json::Value;

use super::MultiPackageBlocks;

/// Remove blocked packages from a `repodata.json`.
///
/// Both `packages` (the `.tar.bz2` generation) and `packages.conda` (the
/// current one) are filtered: a channel serves both for the same release, and
/// leaving either would keep the version installable.
///
/// Returns the `name-version` pairs removed, as the document spelled them.
pub fn strip_repodata(doc: &mut Value, blocked: &MultiPackageBlocks) -> Vec<String> {
    let mut removed = Vec::new();

    for key in ["packages", "packages.conda"] {
        let Some(entries) = doc.get_mut(key).and_then(Value::as_object_mut) else {
            continue;
        };
        let hit: Vec<String> = entries
            .iter()
            .filter(|(_, entry)| {
                let name = entry.get("name").and_then(Value::as_str);
                let version = entry.get("version").and_then(Value::as_str);
                match (name, version) {
                    (Some(n), Some(v)) => blocked.contains(n, v),
                    // An entry this proxy cannot read the coordinate out of is
                    // kept: over-listing one package beats emptying a channel.
                    _ => false,
                }
            })
            .map(|(filename, _)| filename.clone())
            .collect();

        for filename in hit {
            if let Some(entry) = entries.remove(&filename) {
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let version = entry
                    .get("version")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                removed.push(format!("{name}-{version}"));
            }
        }
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(pairs: &[(&str, &str)]) -> MultiPackageBlocks {
        MultiPackageBlocks::new(
            RegistryKind::Conda,
            pairs
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }

    fn repodata() -> Value {
        json!({
            "info": { "subdir": "linux-64" },
            "packages": {
                "numpy-1.24.0-py311_0.tar.bz2": { "name": "numpy", "version": "1.24.0" },
                "numpy-1.25.0-py311_0.tar.bz2": { "name": "numpy", "version": "1.25.0" },
                "scipy-1.11.0-py311_0.tar.bz2": { "name": "scipy", "version": "1.11.0" }
            },
            "packages.conda": {
                "numpy-1.24.0-py311_0.conda": { "name": "numpy", "version": "1.24.0" },
                "numpy-1.25.0-py311_0.conda": { "name": "numpy", "version": "1.25.0" }
            },
            "repodata_version": 1
        })
    }

    fn filenames(doc: &Value, key: &str) -> Vec<String> {
        let mut v: Vec<String> = doc[key]
            .as_object()
            .expect("a package map")
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// A channel serves both generations for the same release, so a block that
    /// only reached one of them would leave the version installable.
    #[test]
    fn a_blocked_package_leaves_both_generations() {
        let mut doc = repodata();
        let removed = strip_repodata(&mut doc, &blocked(&[("numpy", "1.24.0")]));

        assert_eq!(removed.len(), 2, "the .tar.bz2 and the .conda");
        assert_eq!(
            filenames(&doc, "packages"),
            [
                "numpy-1.25.0-py311_0.tar.bz2",
                "scipy-1.11.0-py311_0.tar.bz2"
            ]
        );
        assert_eq!(
            filenames(&doc, "packages.conda"),
            ["numpy-1.25.0-py311_0.conda"]
        );
    }

    /// The blocked set spans the whole channel, so a block has to match on the
    /// *pair*: another package at the same version stays.
    #[test]
    fn a_block_is_scoped_to_its_package_not_to_the_version_string() {
        let mut doc = repodata();
        strip_repodata(&mut doc, &blocked(&[("scipy", "1.24.0")]));

        assert_eq!(
            filenames(&doc, "packages").len(),
            3,
            "scipy has no 1.24.0, and numpy's must not be taken for it"
        );
    }

    #[test]
    fn blocking_an_absent_package_changes_nothing() {
        let mut doc = repodata();
        let before = doc.clone();
        assert!(strip_repodata(&mut doc, &blocked(&[("pandas", "2.0.0")])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn the_channel_envelope_survives() {
        let mut doc = repodata();
        strip_repodata(&mut doc, &blocked(&[("numpy", "1.24.0")]));

        assert_eq!(doc["info"]["subdir"], "linux-64");
        assert_eq!(doc["repodata_version"], 1);
    }

    #[test]
    fn blocking_everything_leaves_well_formed_empty_maps() {
        let mut doc = repodata();
        strip_repodata(
            &mut doc,
            &blocked(&[
                ("numpy", "1.24.0"),
                ("numpy", "1.25.0"),
                ("scipy", "1.11.0"),
            ]),
        );

        assert_eq!(doc["packages"], json!({}));
        assert_eq!(doc["packages.conda"], json!({}));
    }

    #[test]
    fn an_entry_with_no_readable_coordinate_is_kept() {
        let mut doc = json!({ "packages": { "mystery.tar.bz2": { "build": "0" } } });
        let before = doc.clone();
        assert!(strip_repodata(&mut doc, &blocked(&[("numpy", "1.0.0")])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_document_is_returned_unchanged() {
        let mut doc = json!({ "not-repodata": true });
        let before = doc.clone();
        assert!(strip_repodata(&mut doc, &blocked(&[("numpy", "1.0.0")])).is_empty());
        assert_eq!(doc, before);
    }
}
