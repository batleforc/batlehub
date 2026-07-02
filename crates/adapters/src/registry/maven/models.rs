use async_trait::async_trait;
use futures::TryStreamExt;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::super::http_client::{cache_control, percent_encode, to_registry_error};
use super::client::MavenRegistryClient;

// ── XML / HTTP helpers ────────────────────────────────────────────────────────

/// Decode `"groupId:artifactId"` into `("groupId", "artifactId")`.
pub(super) fn decode_name(name: &str) -> Result<(&str, &str), CoreError> {
    name.split_once(':').ok_or_else(|| {
        CoreError::Registry(format!(
            "invalid Maven package name '{name}': expected 'groupId:artifactId'"
        ))
    })
}

/// Extract the text content of the first matching XML tag.
pub(super) fn extract_xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(open.as_str())? + open.len();
    let end = xml[start..].find(close.as_str())?;
    Some(&xml[start..start + end])
}

/// Extract text content of all occurrences of a tag.
pub(super) fn extract_all_xml_tags<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
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
pub(super) fn parse_last_updated(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
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

        let cache_control = cache_control(&resp);

        let stream = resp.bytes_stream().map_err(to_registry_error);

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
        let res = self.get(&url).send().await.map_err(to_registry_error)?;

        if !res.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = res.json().await.map_err(to_registry_error)?;

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
    use super::super::client::MavenRegistryClient;
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
        let _mock_meta = server
            .mock("GET", "/com/example/mylib/maven-metadata.xml")
            .with_status(200)
            .with_header("content-type", "application/xml")
            .with_body(METADATA_XML)
            .create_async()
            .await;
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(200)
            .with_header("Last-Modified", "Fri, 01 Mar 2024 08:00:00 GMT")
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
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
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(200)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
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
        let _mock_head = server
            .mock("HEAD", "/com/example/mylib/1.0.0/mylib-1.0.0.pom")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("maven", "com.example:mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        assert!(meta.published_at.is_some());
    }
}
