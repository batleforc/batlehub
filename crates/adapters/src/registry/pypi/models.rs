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
    /// PEP 345's `Project-URL`, a free-form `label → URL` map. There is no
    /// standard label for "the source", so the ones publishers actually use are
    /// matched case-insensitively by [`Self::repository`].
    #[serde(default)]
    pub(super) project_urls: Option<std::collections::HashMap<String, String>>,
    /// The deprecated `Home-page`, still the only link a great many packages set.
    #[serde(default)]
    pub(super) home_page: Option<String>,
}

impl PypiVersionInfo {
    /// The source-code URL, from whichever label the publisher chose.
    ///
    /// Ordered by how unambiguously the label means "the code": `Source` and
    /// its variants first, then the forge-shaped `Repository`/`Code`. A
    /// `Homepage` label is deliberately not consulted here — it is the homepage,
    /// and it has its own field.
    pub(super) fn repository(&self) -> Option<&str> {
        const LABELS: &[&str] = &[
            "source",
            "source code",
            "sourcecode",
            "repository",
            "repo",
            "code",
            "github",
        ];
        let urls = self.project_urls.as_ref()?;
        LABELS.iter().find_map(|label| {
            urls.iter()
                .find(|(k, _)| k.trim().eq_ignore_ascii_case(label))
                .map(|(_, v)| v.as_str())
        })
    }

    /// The homepage: the `Homepage` project-URL label, or the deprecated
    /// `Home-page` field it replaced.
    pub(super) fn homepage(&self) -> Option<&str> {
        self.project_urls
            .as_ref()
            .and_then(|urls| {
                urls.iter()
                    .find(|(k, _)| k.trim().eq_ignore_ascii_case("homepage"))
                    .map(|(_, v)| v.as_str())
            })
            .or(self.home_page.as_deref())
    }
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
