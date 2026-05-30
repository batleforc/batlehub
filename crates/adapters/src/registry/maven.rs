use async_trait::async_trait;
use futures::TryStreamExt;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, percent_encode, UpstreamHttpOptions};

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
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
    /// Resolved search URL base. `None` = disabled; `Some(url)` = use this.
    search_base: Option<String>,
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

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    fn head(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.head(url);
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
    ///
    /// This gives a per-version publish timestamp, which is more accurate than the
    /// artifact-collection `<lastUpdated>` in `maven-metadata.xml` (which reflects the
    /// most-recently-added version, not the requested one).
    async fn head_pom_last_modified(
        &self,
        pkg: &PackageId,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
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

    async fn fetch_metadata_xml(
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

/// Parse an HTTP `Last-Modified` header value (RFC 7231 / RFC 2822) into a `DateTime<Utc>`.
fn parse_http_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
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

        // For specific versions, HEAD the .pom file to get a per-version Last-Modified
        // timestamp (more accurate than the artifact-collection <lastUpdated>).
        // Fall back to <lastUpdated> if the HEAD request fails or returns no date.
        let published_at = if pkg.version != "maven-metadata.xml" {
            self.head_pom_last_modified(pkg)
                .await
                .or_else(|| extract_xml_tag(&xml, "lastUpdated").and_then(parse_last_updated))
        } else {
            extract_xml_tag(&xml, "lastUpdated").and_then(parse_last_updated)
        };

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

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(serde::Deserialize)]
        struct SearchResponse {
            response: SolrResponse,
        }
        #[derive(serde::Deserialize)]
        struct SolrResponse {
            docs: Vec<SolrDoc>,
        }
        #[derive(serde::Deserialize)]
        struct SolrDoc {
            // "g:a" combined identifier
            id: String,
            #[serde(rename = "latestVersion")]
            latest_version: Option<String>,
        }

        let Some(ref search_base) = self.search_base else {
            return Ok(vec![]);
        };
        let url = format!(
            "{}/solrsearch/select?q={}&rows={}&wt=json",
            search_base,
            percent_encode(query),
            limit.min(50),
        );
        let res = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = res
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(body
            .response
            .docs
            .into_iter()
            .map(|d| UpstreamPackage {
                name: d.id,
                latest_version: d.latest_version.unwrap_or_else(|| "unknown".to_string()),
                description: None,
            })
            .collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    const METADATA_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>com.example</groupId>
  <artifactId>mylib</artifactId>
  <versioning>
    <release>1.2.0</release>
    <latest>1.2.0</latest>
    <versions>
      <version>1.0.0</version>
      <version>1.2.0</version>
    </versions>
    <lastUpdated>20240315143022</lastUpdated>
  </versioning>
</metadata>"#;

    fn client(base_url: &str) -> MavenRegistryClient {
        MavenRegistryClient::new(base_url, &Default::default()).unwrap()
    }

    #[tokio::test]
    async fn resolve_metadata_metadata_xml_uses_last_updated() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/com/example/mylib/maven-metadata.xml")
            .with_status(200)
            .with_header("content-type", "application/xml")
            .with_body(METADATA_XML)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "maven-metadata.xml");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        // For metadata-xml requests: uses <lastUpdated>
        assert!(
            meta.published_at.is_some(),
            "published_at should be set from lastUpdated"
        );
        let ts = meta.published_at.unwrap();
        assert_eq!(ts.format("%Y-%m-%d").to_string(), "2024-03-15");
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version_uses_pom_last_modified() {
        let mut server = Server::new_async().await;
        // metadata XML is still fetched (for cache-control / latest version)
        let _mock_meta = server
            .mock("GET", "/com/example/mylib/maven-metadata.xml")
            .with_status(200)
            .with_header("content-type", "application/xml")
            .with_body(METADATA_XML)
            .create_async()
            .await;
        // HEAD request for the POM file returns Last-Modified
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(200)
            .with_header("Last-Modified", "Fri, 01 Mar 2024 08:00:00 GMT")
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        // For specific-version requests: uses POM Last-Modified (more accurate)
        assert!(
            meta.published_at.is_some(),
            "published_at should be set from POM Last-Modified"
        );
        let ts = meta.published_at.unwrap();
        assert_eq!(ts.format("%Y-%m-%d").to_string(), "2024-03-01");
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version_falls_back_to_last_updated() {
        let mut server = Server::new_async().await;
        let _mock_meta = server
            .mock("GET", "/com/example/mylib/maven-metadata.xml")
            .with_status(200)
            .with_header("content-type", "application/xml")
            .with_body(METADATA_XML)
            .create_async()
            .await;
        // HEAD returns no Last-Modified header
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(200)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        // Falls back to <lastUpdated> from metadata XML
        assert!(
            meta.published_at.is_some(),
            "should fall back to lastUpdated"
        );
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version_pom_head_404_falls_back() {
        let mut server = Server::new_async().await;
        let _mock_meta = server
            .mock("GET", "/com/example/mylib/maven-metadata.xml")
            .with_status(200)
            .with_header("content-type", "application/xml")
            .with_body(METADATA_XML)
            .create_async()
            .await;
        // HEAD for POM returns 404 (e.g. odd repo layout)
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        // Falls back to <lastUpdated>
        assert!(meta.published_at.is_some());
    }

    #[test]
    fn parse_http_date_valid() {
        let dt = parse_http_date("Fri, 15 Mar 2024 12:34:56 GMT").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2024-03-15 12:34:56"
        );
    }

    #[test]
    fn parse_http_date_invalid_returns_none() {
        assert!(parse_http_date("not-a-date").is_none());
    }
}
