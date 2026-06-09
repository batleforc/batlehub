use batlehub_core::{entities::PackageId, error::CoreError};

use super::super::http_client::{apply_upstream_options, UpstreamHttpOptions};
use super::models::decode_name;

/// Maven Central-compatible registry client.
///
/// Proxies any Maven repository that follows the standard Maven 2 layout:
/// `{base_url}/{group/path}/{artifactId}/{version}/{filename}`
///
/// Default upstream: `https://repo1.maven.org/maven2`
///
/// Supported `PackageId` conventions:
/// - `name`    = `"{groupId}:{artifactId}"` (e.g. `"com.google.guava:guava"`)
/// - `version` = version string (e.g. `"32.0.1-jre"`) **or** `"maven-metadata.xml"` to
///   fetch the artifact-level metadata document
/// - `artifact`:
///   - `None` → `.pom` file for that version (or the metadata XML when
///     `version == "maven-metadata.xml"`)
///   - `Some(filename)` → the exact filename to fetch from the version directory
///     (e.g. `"guava-32.0.1-jre.jar"`, `"guava-32.0.1-jre.jar.sha1"`)
pub struct MavenRegistryClient {
    pub(super) http: reqwest::Client,
    pub(super) base_url: String,
    pub(super) basic_auth: Option<(String, String)>,
    /// Resolved search URL base. `None` = disabled; `Some(url)` = use this.
    pub(super) search_base: Option<String>,
}

impl MavenRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;

        // Resolve the search base URL:
        //   - explicit empty string → disabled
        //   - explicit non-empty URL → use as-is
        //   - absent → default to Maven Central's search service
        let search_base = match opts.search_url.as_deref() {
            Some("") => None,
            Some(url) => Some(url.trim_end_matches('/').to_owned()),
            None => Some("https://search.maven.org".to_owned()),
        };

        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
            search_base,
        })
    }

    pub(super) fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    pub(super) fn head(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.head(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    /// Build the upstream URL for the given `PackageId`.
    pub(super) fn artifact_url(&self, pkg: &PackageId) -> Result<String, CoreError> {
        let (group_id, artifact_id) = decode_name(&pkg.name)?;
        let group_path = group_id.replace('.', "/");
        let base = self.base_url.trim_end_matches('/');

        if pkg.version == "maven-metadata.xml" {
            Ok(format!(
                "{base}/{group_path}/{artifact_id}/maven-metadata.xml"
            ))
        } else {
            let filename = match pkg.artifact.as_deref() {
                Some(f) => f.to_owned(),
                None => format!("{artifact_id}-{}.pom", pkg.version),
            };
            Ok(format!(
                "{base}/{group_path}/{artifact_id}/{}/{filename}",
                pkg.version
            ))
        }
    }

    /// HEAD-request the `.pom` file for a specific version and return its `Last-Modified` timestamp.
    pub(super) async fn head_pom_last_modified(
        &self,
        pkg: &PackageId,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        use super::models::parse_http_date;
        let (group_id, artifact_id) = decode_name(&pkg.name).ok()?;
        let group_path = group_id.replace('.', "/");
        let base = self.base_url.trim_end_matches('/');
        let url = format!(
            "{base}/{group_path}/{artifact_id}/{}/{artifact_id}-{}.pom",
            pkg.version, pkg.version
        );
        let resp = self.head(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_http_date)
    }

    pub(super) async fn fetch_metadata_xml(
        &self,
        pkg: &PackageId,
    ) -> Result<(String, Option<String>), CoreError> {
        let (group_id, artifact_id) = decode_name(&pkg.name)?;
        let group_path = group_id.replace('.', "/");
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/{group_path}/{artifact_id}/maven-metadata.xml");

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("Maven metadata request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "Maven metadata not found for {}:{}",
                group_id, artifact_id
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "Maven metadata request returned {}: {artifact_id}",
                resp.status()
            )));
        }

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let body = resp
            .text()
            .await
            .map_err(|e| CoreError::Registry(format!("reading Maven metadata: {e}")))?;

        Ok((body, cache_control))
    }
}
