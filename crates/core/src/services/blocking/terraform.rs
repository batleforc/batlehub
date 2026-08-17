//! Terraform: module and provider version listings.
//!
//! One route (`/v1/{namespace}/versions`) serves two document shapes, told
//! apart by the package name's `modules/` or `providers/` prefix — the same
//! discrimination `TerraformRegistryClient::artifact_url` already makes:
//!
//! ```text
//! modules/…    {"modules":  [{"source": …, "versions": [{"version": …}, …]}]}
//! providers/…  {"id": …,    "versions": [{"version": …, "platforms": […]}, …]}
//! ```
//!
//! Neither document names a preferred version — the client picks from the list
//! against its own constraint — so there is nothing to repair beyond removing
//! the entries.

use serde_json::Value;

use super::BlockedVersions;

/// Remove blocked versions from a module or provider version listing.
///
/// Shape-directed rather than name-directed: whichever of the two layouts the
/// document has is the one filtered, so a caller that mislabels a module as a
/// provider still gets the right answer instead of a silently unfiltered
/// document.
pub fn strip_versions(doc: &mut Value, blocked: &BlockedVersions) -> Vec<String> {
    let mut removed = Vec::new();

    // Providers: a flat `versions` array at the top level.
    if let Some(versions) = doc.get_mut("versions").and_then(Value::as_array_mut) {
        retain_allowed(versions, blocked, &mut removed);
    }

    // Modules: `versions` nested one level down, under each `modules` entry.
    if let Some(modules) = doc.get_mut("modules").and_then(Value::as_array_mut) {
        for module in modules.iter_mut() {
            if let Some(versions) = module.get_mut("versions").and_then(Value::as_array_mut) {
                retain_allowed(versions, blocked, &mut removed);
            }
        }
    }

    removed
}

fn retain_allowed(versions: &mut Vec<Value>, blocked: &BlockedVersions, removed: &mut Vec<String>) {
    versions.retain(|entry| {
        let Some(v) = entry.get("version").and_then(Value::as_str) else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::RegistryKind;
    use serde_json::json;

    fn blocked(vs: &[&str]) -> BlockedVersions {
        BlockedVersions::new(
            RegistryKind::Terraform,
            vs.iter().map(|s| (*s).to_owned()).collect(),
        )
    }

    fn provider_doc() -> Value {
        json!({
            "id": "hashicorp/aws",
            "versions": [
                { "version": "5.0.0", "protocols": ["5.0"] },
                { "version": "5.1.0", "protocols": ["5.0"] }
            ]
        })
    }

    fn module_doc() -> Value {
        json!({
            "modules": [{
                "source": "terraform-aws-modules/vpc/aws",
                "versions": [{ "version": "5.0.0" }, { "version": "5.1.0" }]
            }]
        })
    }

    #[test]
    fn provider_versions_drop_the_blocked_entry() {
        let mut doc = provider_doc();
        let removed = strip_versions(&mut doc, &blocked(&["5.1.0"]));

        assert_eq!(removed, vec!["5.1.0".to_owned()]);
        assert_eq!(
            doc["versions"],
            json!([{ "version": "5.0.0", "protocols": ["5.0"] }])
        );
    }

    #[test]
    fn module_versions_drop_the_blocked_entry_one_level_down() {
        let mut doc = module_doc();
        let removed = strip_versions(&mut doc, &blocked(&["5.0.0"]));

        assert_eq!(removed, vec!["5.0.0".to_owned()]);
        assert_eq!(
            doc["modules"][0]["versions"],
            json!([{ "version": "5.1.0" }])
        );
        assert_eq!(
            doc["modules"][0]["source"],
            json!("terraform-aws-modules/vpc/aws"),
            "the envelope survives"
        );
    }

    #[test]
    fn blocking_every_version_leaves_a_well_formed_empty_listing() {
        let mut doc = module_doc();
        strip_versions(&mut doc, &blocked(&["5.0.0", "5.1.0"]));

        assert_eq!(doc["modules"][0]["versions"], json!([]));
    }

    #[test]
    fn blocking_an_absent_version_changes_nothing() {
        let mut doc = provider_doc();
        let before = doc.clone();
        assert!(strip_versions(&mut doc, &blocked(&["9.9.9"])).is_empty());
        assert_eq!(doc, before);
    }

    #[test]
    fn a_malformed_document_is_returned_unchanged() {
        let mut doc = json!({ "errors": ["Not Found"] });
        let before = doc.clone();
        assert!(strip_versions(&mut doc, &blocked(&["1.0.0"])).is_empty());
        assert_eq!(doc, before);
    }
}
