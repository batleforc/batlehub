use super::Deserialize;

#[derive(Deserialize)]
pub(super) struct PypiSearchInfo {
    pub(super) info: PypiSearchInfoInner,
}

#[derive(Deserialize)]
pub(super) struct PypiSearchInfoInner {
    pub(super) name: String,
    pub(super) version: String,
    pub(super) summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PypiVersionJson {
    #[serde(default)]
    pub(super) urls: Vec<PypiFileInfo>,
    /// The long description and its declared markup.
    ///
    /// Per-version by construction — a wheel's `METADATA` ships inside the
    /// wheel — which is why PyPI is metadata-borne and can answer for a version
    /// this instance holds no bytes for (RFC 0007 §4.3).
    #[serde(default)]
    pub(super) info: Option<PypiVersionInfo>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct PypiVersionInfo {
    #[serde(default)]
    pub(super) description: Option<String>,
    /// PEP 566's `Description-Content-Type`. Written by the publisher in the
    /// package metadata, not a transport header — which is why it is trusted to
    /// choose a renderer and an upstream `Content-Type` is not (§7.4).
    #[serde(default)]
    pub(super) description_content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PypiPackageJson {
    #[serde(default)]
    pub(super) releases: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PypiFileInfo {
    pub(super) filename: String,
    pub(super) url: String,
    #[serde(default)]
    pub(super) digests: PypiDigests,
    #[serde(default)]
    pub(super) upload_time_iso_8601: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct PypiDigests {
    #[serde(default)]
    pub(super) sha256: Option<String>,
}
