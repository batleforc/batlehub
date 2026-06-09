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
