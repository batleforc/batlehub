use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient, UpstreamPackage},
};

use super::http_client::{apply_upstream_options, percent_encode, UpstreamHttpOptions};

/// NuGet v3 protocol registry client.
///
/// Proxies any NuGet v3-compatible server (nuget.org, Azure Artifacts, GitHub Packages, etc.).
///
/// Default upstream: `https://api.nuget.org`
///
/// `PackageId` conventions:
/// - `name`: package ID lower-cased (e.g. `"newtonsoft.json"`)
/// - `version`:
///   - `"__index__"` → fetch flat-container version list (`/v3-flatcontainer/{id}/index.json`)
///   - `"__registration__"` → fetch registration metadata (`/v3/registration5/{id}/index.json`)
///   - version string (e.g. `"13.0.3"`) → specific version artifact
/// - `artifact`:
///   - `None` → default `.nupkg` for that version, or the special JSON when version is a sentinel
///   - `Some(filename)` → exact filename (e.g. `"newtonsoft.json.13.0.3.nupkg"`, `"*.nuspec"`)
pub struct NugetRegistryClient {
    http: reqwest::Client,
    /// `{base}/v3-flatcontainer` — package downloads and version lists.
    flat_url: String,
    /// `{base}/v3/registration5` — rich metadata (dependencies, authors, …).
    reg_url: String,
    /// Optional NuGet search API base URL.
    search_base: Option<String>,
    /// `X-NuGet-ApiKey` / Bearer token for the upstream, if configured.
    api_key: Option<String>,
}

impl NugetRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .user_agent("batlehub/0.1")
            .redirect(reqwest::redirect::Policy::limited(10));
        let http = apply_upstream_options(builder, opts)?;

        let base = base_url.into();
        let base = base.trim_end_matches('/').to_owned();

        // For nuget.org the flat container lives at a different host than the API.
        let (flat_url, reg_url) = if base == "https://api.nuget.org" {
            (
                "https://api.nuget.org/v3-flatcontainer".to_owned(),
                "https://api.nuget.org/v3/registration5".to_owned(),
            )
        } else {
            (
                format!("{base}/v3-flatcontainer"),
                format!("{base}/v3/registration5"),
            )
        };

        // Search base: explicit config wins; fall back to nuget.org search for nuget.org upstream.
        let search_base = match opts.search_url.as_deref() {
            Some("") => None,
            Some(url) => Some(url.trim_end_matches('/').to_owned()),
            None if base == "https://api.nuget.org" => {
                Some("https://azuresearch-usnc.nuget.org/query".to_owned())
            }
            None => None,
        };

        // Bearer token used as X-NuGet-ApiKey for the upstream.
        let api_key = opts.bearer_token.clone();

        Ok(Self {
            http,
            flat_url,
            reg_url,
            search_base,
            api_key,
        })
    }

    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.request(method, url);
        match &self.api_key {
            Some(key) => rb.header("X-NuGet-ApiKey", key),
            None => rb,
        }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::GET, url)
    }

    fn head(&self, url: &str) -> reqwest::RequestBuilder {
        self.request(reqwest::Method::HEAD, url)
    }

    /// `GET {flat_url}/{id}/index.json` → `{"versions":[…]}`
    async fn fetch_flat_index(
        &self,
        id: &str,
    ) -> Result<(Vec<String>, Option<String>), CoreError> {
        #[derive(Deserialize)]
        struct FlatIndex {
            versions: Vec<String>,
        }

        let url = format!("{}/{}/index.json", self.flat_url, id);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("NuGet flat index request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "NuGet package '{id}' not found on upstream"
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "NuGet flat index returned {} for '{id}'",
                resp.status()
            )));
        }

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        let body: FlatIndex = resp
            .json()
            .await
            .map_err(|e| CoreError::Registry(format!("NuGet flat index parse error: {e}")))?;

        Ok((body.versions, cache_control))
    }

    /// Fetch a URL and return only its `cache-control` header value.
    ///
    /// Used for sentinel versions (`__index__`, `__registration__`) where the body
    /// is irrelevant to metadata resolution — we just need to confirm the resource
    /// exists and capture its caching hint.
    async fn fetch_cache_control(
        &self,
        url: &str,
        label: &str,
    ) -> Result<Option<String>, CoreError> {
        let resp = self
            .get(url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("NuGet {label} request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "NuGet {label} not found on upstream"
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "NuGet {label} returned {} from upstream",
                resp.status()
            )));
        }

        Ok(resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned))
    }

    /// HEAD the `.nupkg` artifact URL to get its `Last-Modified` timestamp.
    async fn head_nupkg_last_modified(
        &self,
        id: &str,
        version: &str,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        let url = format!("{}/{}/{}/{}.{}.nupkg", self.flat_url, id, version, id, version);
        let resp = self.head(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_http_date)
    }
}

fn parse_http_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(s.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Normalise a NuGet package ID to lower-case (IDs are case-insensitive in the protocol).
pub fn normalize_id(id: &str) -> String {
    id.to_lowercase()
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for NugetRegistryClient {
    fn registry_type(&self) -> &str {
        "nuget"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let id = &pkg.name; // already lowercased by the handler

        match pkg.version.as_str() {
            "__index__" => {
                let url = format!("{}/{}/index.json", self.flat_url, id);
                let cache_control = self
                    .fetch_cache_control(&url, &format!("flat index for '{id}'"))
                    .await?;
                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }

            "__registration__" => {
                let url = format!("{}/{}/index.json", self.reg_url, id);
                let cache_control = self
                    .fetch_cache_control(&url, &format!("registration for '{id}'"))
                    .await?;
                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at: None,
                    download_url: None,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }

            version => {
                // Specific version: confirm it exists in the flat container, get timestamp.
                let (versions, cache_control) = self.fetch_flat_index(id).await?;
                if !versions.iter().any(|v| v == version) {
                    return Err(CoreError::NotFound(format!(
                        "NuGet package '{id}' version '{version}' not found"
                    )));
                }

                let published_at = self.head_nupkg_last_modified(id, version).await;

                let download_url = Some(format!(
                    "{}/{}/{}/{}.{}.nupkg",
                    self.flat_url, id, version, id, version
                ));

                Ok(PackageMetadata {
                    id: pkg.clone(),
                    published_at,
                    download_url,
                    checksum: None,
                    is_signed: None,
                    extra: serde_json::Value::Null,
                    cache_control,
                })
            }
        }
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let id = &pkg.name;
        let url = match (pkg.version.as_str(), pkg.artifact.as_deref()) {
            ("__index__", _) => format!("{}/{}/index.json", self.flat_url, id),
            ("__registration__", _) => format!("{}/{}/index.json", self.reg_url, id),
            (version, Some(filename)) => {
                format!("{}/{}/{}/{}", self.flat_url, id, version, filename)
            }
            (version, None) => {
                format!("{}/{}/{}/{}.{}.nupkg", self.flat_url, id, version, id, version)
            }
        };

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("NuGet artifact request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "NuGet artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "NuGet artifact returned {} for {}",
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
        let id = normalize_id(package);
        let (mut versions, _) = self.fetch_flat_index(&id).await?;
        versions.sort();
        Ok(versions)
    }

    async fn search_packages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UpstreamPackage>, CoreError> {
        #[derive(Deserialize)]
        struct SearchResponse {
            data: Vec<SearchEntry>,
        }
        #[derive(Deserialize)]
        struct SearchEntry {
            id: String,
            version: String,
            description: Option<String>,
        }

        let Some(ref search_base) = self.search_base else {
            return Ok(vec![]);
        };

        let url = format!(
            "{}?q={}&take={}&prerelease=false",
            search_base,
            percent_encode(query),
            limit.min(100),
        );

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(vec![]);
        }

        let body: SearchResponse = resp
            .json()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        Ok(body
            .data
            .into_iter()
            .map(|e| UpstreamPackage {
                name: e.id,
                latest_version: e.version,
                description: e.description,
            })
            .collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    fn client(base_url: &str) -> NugetRegistryClient {
        NugetRegistryClient::new(base_url, &Default::default()).unwrap()
    }

    #[tokio::test]
    async fn list_versions_returns_sorted() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v3-flatcontainer/newtonsoft.json/index.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"versions":["13.0.3","12.0.1","13.0.1"]}"#)
            .create_async()
            .await;

        let c = client(&server.url());
        let versions = c.list_versions("Newtonsoft.Json").await.unwrap();
        assert_eq!(versions, vec!["12.0.1", "13.0.1", "13.0.3"]);
    }

    #[tokio::test]
    async fn resolve_metadata_index_sentinel() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v3-flatcontainer/mylib/index.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_header("cache-control", "max-age=3600")
            .with_body(r#"{"versions":["1.0.0"]}"#)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("nuget", "mylib", "__index__");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.cache_control.as_deref(), Some("max-age=3600"));
    }

    #[tokio::test]
    async fn resolve_metadata_specific_version_found() {
        let mut server = Server::new_async().await;
        let _mock_index = server
            .mock("GET", "/v3-flatcontainer/mylib/index.json")
            .with_status(200)
            .with_body(r#"{"versions":["1.0.0","1.1.0"]}"#)
            .create_async()
            .await;
        let _mock_head = server
            .mock("HEAD", "/v3-flatcontainer/mylib/1.0.0/mylib.1.0.0.nupkg")
            .with_status(200)
            .with_header("last-modified", "Fri, 01 Mar 2024 08:00:00 GMT")
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("nuget", "mylib", "1.0.0");
        let meta = c.resolve_metadata(&pkg).await.unwrap();
        assert!(meta.published_at.is_some());
        assert!(meta.download_url.is_some());
        let url = meta.download_url.unwrap();
        assert!(url.ends_with("mylib.1.0.0.nupkg"));
    }

    #[tokio::test]
    async fn resolve_metadata_version_not_found() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v3-flatcontainer/mylib/index.json")
            .with_status(200)
            .with_body(r#"{"versions":["1.0.0"]}"#)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("nuget", "mylib", "9.9.9");
        let err = c.resolve_metadata(&pkg).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn resolve_metadata_package_404() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v3-flatcontainer/unknown/index.json")
            .with_status(404)
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("nuget", "unknown", "__index__");
        let err = c.resolve_metadata(&pkg).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn fetch_artifact_nupkg() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                "/v3-flatcontainer/mylib/1.0.0/mylib.1.0.0.nupkg",
            )
            .with_status(200)
            .with_header("content-type", "application/octet-stream")
            .with_body(b"FAKEZIP".as_ref())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg = PackageId::new("nuget", "mylib", "1.0.0");
        let artifact = c.fetch_artifact(&pkg).await.unwrap();
        use futures::StreamExt;
        let mut buf = Vec::new();
        let mut stream = artifact.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(buf, b"FAKEZIP");
    }

    #[tokio::test]
    async fn fetch_artifact_named_file() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/v3-flatcontainer/mylib/1.0.0/mylib.1.0.0.nuspec")
            .with_status(200)
            .with_body(b"<package/>".as_ref())
            .create_async()
            .await;

        let c = client(&server.url());
        let pkg =
            PackageId::new("nuget", "mylib", "1.0.0").with_artifact("mylib.1.0.0.nuspec");
        let artifact = c.fetch_artifact(&pkg).await.unwrap();
        use futures::StreamExt;
        let mut buf = Vec::new();
        let mut stream = artifact.stream;
        while let Some(chunk) = stream.next().await {
            buf.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(buf, b"<package/>");
    }

    #[tokio::test]
    async fn search_packages_parses_response() {
        let mut server = Server::new_async().await;
        let _mock = server
            .mock("GET", "/query?q=json&take=10&prerelease=false")
            .with_status(200)
            .with_body(
                r#"{"data":[{"id":"Newtonsoft.Json","version":"13.0.3","description":"JSON framework"}]}"#,
            )
            .create_async()
            .await;

        let base = server.url();
        // Custom search URL pointing to mock server
        let opts = UpstreamHttpOptions {
            search_url: Some(format!("{}/query", base)),
            ..Default::default()
        };
        let c = NugetRegistryClient::new(&base, &opts).unwrap();
        let results = c.search_packages("json", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Newtonsoft.Json");
        assert_eq!(results[0].latest_version, "13.0.3");
        assert_eq!(results[0].description.as_deref(), Some("JSON framework"));
    }

    #[test]
    fn normalize_id_lowercases() {
        assert_eq!(normalize_id("Newtonsoft.Json"), "newtonsoft.json");
        assert_eq!(normalize_id("MYLIB"), "mylib");
    }

    #[test]
    fn parse_http_date_valid() {
        let dt = parse_http_date("Fri, 15 Mar 2024 12:34:56 GMT").unwrap();
        assert_eq!(dt.format("%Y-%m-%d").to_string(), "2024-03-15");
    }
}
