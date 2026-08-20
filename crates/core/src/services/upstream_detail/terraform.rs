//! Terraform: module and provider versions.
//!
//! Two shapes from one endpoint family: providers answer with a top-level
//! `versions` array, modules with `versions` nested under each `modules` entry.
//! Both are read, because the explore name carries the prefix that decides which
//! and this reader does not need to know it.

use super::{json, UpstreamDetail, UpstreamVersion};
use crate::ports::VersionDocument;

pub(super) fn read(doc: &VersionDocument) -> UpstreamDetail {
    let Some(root) = json(doc) else {
        return UpstreamDetail::default();
    };
    let mut versions = Vec::new();
    collect(root.get("versions"), &mut versions);
    if let Some(modules) = root.get("modules").and_then(|m| m.as_array()) {
        for module in modules {
            collect(module.get("versions"), &mut versions);
        }
    }
    UpstreamDetail {
        versions,
        readmes: Default::default(),
        // `source` is on the module *detail* document, not on the version list.
        links: None,
    }
}

fn collect(value: Option<&serde_json::Value>, out: &mut Vec<UpstreamVersion>) {
    let Some(list) = value.and_then(|v| v.as_array()) else {
        return;
    };
    out.extend(
        list.iter()
            .filter_map(|entry| entry.get("version")?.as_str())
            .map(UpstreamVersion::bare),
    );
}
