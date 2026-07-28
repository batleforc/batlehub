//! JetBrains Marketplace (plugins.jetbrains.com) handlers.
//!
//! Emulates the IDE-facing marketplace surface (search, compatible-updates,
//! meta.json blobs, downloads) plus the `updatePlugins.xml` custom-repo XML and
//! a marketplace-compatible multipart publish, across local/hybrid/proxy modes.
//!
//! Offline resilience: per-plugin endpoints render from the ProxyService
//! metadata cache (`resolve_metadata_for`, stale-capable), artifacts go through
//! the ProxyService storage cache, and query/list endpoints use the
//! [`cached_forward`] body cache — anything seen once keeps working after
//! upstream loss.

pub mod cached_forward;
mod files;
mod ide;
mod plugin_archive;
mod publish;
mod render;
mod xml;

pub use files::{
    jbm_broken_plugins, jbm_file_download, jbm_ide_extensions, jbm_jb_plugins_xml_ids,
    jbm_plugin_download, jbm_plugin_manager, jbm_plugin_meta, jbm_plugins_xml_ids, jbm_update_meta,
};
pub use ide::{
    jbm_aggregation, jbm_comments, jbm_compatible_updates, jbm_feature_implementations,
    jbm_plugin_info, jbm_plugin_updates, jbm_search_plugins, jbm_search_plugins_ide,
};
pub use publish::jbm_upload;
pub use xml::{jbm_plugins_list, jbm_update_plugins_xml};

use actix_web::HttpRequest;

use crate::{error::AppError, RegistryMap};

pub(crate) const REGISTRY_TYPE: &str = "jetbrains-marketplace";

/// The Stable channel is the empty string, matching marketplace conventions.
pub(crate) const STABLE_CHANNEL: &str = "";

pub(crate) fn require_jbm(registry: &str, map: &RegistryMap) -> Result<(), AppError> {
    super::common::require_registry_type(registry, REGISTRY_TYPE, map)
}

/// Absolute base URL of this proxy as seen by the caller, for self-referencing
/// download URLs in generated XML/JSON (inline `connection_info` pattern, cf.
/// the PyPI simple index).
pub(crate) fn proxy_base(req: &HttpRequest) -> String {
    let conn_info = req.connection_info();
    format!("{}://{}", conn_info.scheme(), conn_info.host())
}

pub(crate) fn content_type_for(file_name: &str) -> &'static str {
    if file_name.ends_with(".zip") {
        "application/zip"
    } else if file_name.ends_with(".jar") {
        "application/java-archive"
    } else if file_name.ends_with(".xml") {
        "application/xml"
    } else if file_name.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::content_type_for;

    #[test]
    fn content_types() {
        assert_eq!(content_type_for("p.zip"), "application/zip");
        assert_eq!(content_type_for("p.jar"), "application/java-archive");
        assert_eq!(content_type_for("updatePlugins.xml"), "application/xml");
        assert_eq!(content_type_for("meta.json"), "application/json");
        assert_eq!(content_type_for("blob"), "application/octet-stream");
    }
}
