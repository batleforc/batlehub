use super::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct TfProviderVersions {
    pub(super) versions: Vec<TfProviderVersion>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TfProviderVersion {
    pub(super) version: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TfModuleVersions {
    pub(super) modules: Vec<TfModuleEntry>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TfModuleEntry {
    pub(super) versions: Vec<TfModuleVersion>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TfModuleVersion {
    pub(super) version: String,
}

/// Shape shared by both the module detail endpoint (official spec) and the provider
/// detail endpoint (supported by registry.terraform.io, not in the official spec).
#[derive(Debug, Deserialize)]
pub(super) struct TfVersionDetail {
    #[serde(default)]
    pub(super) published_at: Option<String>,
}

/// Returned by `GET /v1/modules/search?q=...`
#[derive(Deserialize)]
pub(super) struct ModuleSearch {
    pub(super) modules: Vec<ModuleHit>,
}

#[derive(Deserialize)]
pub(super) struct ModuleHit {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) provider: String,
    pub(super) version: String,
    pub(super) description: Option<String>,
}

/// Returned by `GET /v1/providers/{namespace}` and `GET /v1/providers/{ns}/{name}/versions`
#[derive(Deserialize)]
pub(super) struct ProviderList {
    #[serde(default)]
    pub(super) providers: Vec<ProviderHit>,
}

#[derive(Deserialize)]
pub(super) struct ProviderHit {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) version: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
}
