use async_trait::async_trait;
use futures::TryStreamExt;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

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
///               fetch the artifact-level metadata document
/// - `artifact`:
///   - `None`          → `.pom` file for that version (or the metadata XML when
///                        `version == "maven-metadata.xml"`)
///   - `Some(filename)` → the exact filename to fetch from the version directory
///                        (e.g. `"guava-32.0.1-jre.jar"`, `"guava-32.0.1-jre.jar.sha1"`)
pub struct MavenRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
}

impl MavenRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self { http, base_url: base_url.into(), basic_auth: opts.basic_auth.clone() })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    /// Build the upstream URL for the given `PackageId`.
    fn artifact_url(&self, pkg: &PackageId) -> Result<String, CoreError> {
        let (group_id, artifact_id) = decode_name(&pkg.name)?;
        let group_path = group_id.replace('.', "/");
        let base = self.base_url.trim_end_matches('/');

        if pkg.version == "maven-metadata.xml" {
            Ok(format!("{base}/{group_path}/{artifact_id}/maven-metadata.xml"))
        } else {
            let filename = match pkg.artifact.as_deref() {
                Some(f) => f.to_owned(),
                None => format!("{artifact_id}-{}.pom", pkg.version),
            };
            Ok(format!("{base}/{group_path}/{artifact_id}/{}/{filename}", pkg.version))
        }
    }

    async fn fetch_metadata_xml(&self, pkg: &PackageId) -> Result<(String, Option<String>), CoreError> {
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

// ── XML helpers ───────────────────────────────────────────────────────────────

/// Decode `"groupId:artifactId"` into `("groupId", "artifactId")`.
fn decode_name(name: &str) -> Result<(&str, &str), CoreError> {
    name.split_once(':').ok_or_else(|| {
        CoreError::Registry(format!(
            "invalid Maven package name '{name}': expected 'groupId:artifactId'"
        ))
    })
}

/// Extract the text content of the first matching XML tag.
fn extract_xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(open.as_str())? + open.len();
    let end = xml[start..].find(close.as_str())?;
    Some(&xml[start..start + end])
}

/// Extract text content of all occurrences of a tag.
fn extract_all_xml_tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut result = Vec::new();
    let mut pos = 0;
    while let Some(rel_start) = xml[pos..].find(open.as_str()) {
        let abs_start = pos + rel_start + open.len();
        if let Some(rel_end) = xml[abs_start..].find(close.as_str()) {
            result.push(&xml[abs_start..abs_start + rel_end]);
            pos = abs_start + rel_end + close.len();
        } else {
            break;
        }
    }
    result
}

/// Parse the `<lastUpdated>` value (format: `yyyyMMddHHmmss`) into a `DateTime<Utc>`.
fn parse_last_updated(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y%m%d%H%M%S")
        .ok()
        .map(|dt| dt.and_utc())
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for MavenRegistryClient {
    fn registry_type(&self) -> &str {
        "maven"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let (xml, cache_control) = self.fetch_metadata_xml(pkg).await?;

        let latest_version = extract_xml_tag(&xml, "release")
            .or_else(|| extract_xml_tag(&xml, "latest"))
            .unwrap_or_default()
            .to_owned();

        let published_at = extract_xml_tag(&xml, "lastUpdated")
            .and_then(parse_last_updated);

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
            download_url: None,
            checksum: None,
            is_signed: None,
            extra: serde_json::json!({ "latest_version": latest_version }),
            cache_control,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let url = self.artifact_url(pkg)?;

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("Maven artifact request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "Maven artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "Maven artifact request returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let stream = resp
            .bytes_stream()
            .map_err(|e| CoreError::Registry(e.to_string()));

        Ok(FetchedArtifact {
            stream: Box::pin(stream),
            cache_control,
        })
    }

    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let pkg = PackageId::new("", package, "maven-metadata.xml");
        let (xml, _) = self.fetch_metadata_xml(&pkg).await?;

        let mut versions: Vec<String> = extract_all_xml_tags(&xml, "version")
            .into_iter()
            .map(str::to_owned)
            .collect();

        // Maven metadata lists versions in ascending order by convention,
        // but some repositories serve them in arbitrary order; sort them.
        versions.sort();
        Ok(versions)
    }
}
