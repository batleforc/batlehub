use async_trait::async_trait;
use futures::TryStreamExt;
use serde::Deserialize;

use batlehub_core::{
    entities::{PackageId, PackageMetadata},
    error::CoreError,
    ports::{FetchedArtifact, RegistryClient},
};

use super::http_client::{apply_upstream_options, UpstreamHttpOptions};

/// Conda channel proxy client.
///
/// Proxies a single conda channel (e.g. `conda-forge`) across all platforms.
///
/// Default upstream: `https://conda.anaconda.org`
///
/// `PackageId` conventions (repodata):
/// - `name`: `"repodata"` for the channel index, or the package filename stem
/// - `version`: platform string (e.g. `"linux-64"`, `"noarch"`)
/// - `artifact`: `None` for repodata, `Some("<filename>")` for a specific package
///
/// `list_versions` is implemented by fetching `repodata.json` for each of the
/// `list_platforms` (default: `noarch` + the four major binary platforms) and
/// collecting every distinct version string for the named package.
pub struct CondaRegistryClient {
    http: reqwest::Client,
    base_url: String,
    basic_auth: Option<(String, String)>,
    /// Platforms queried when `list_versions` is called.
    /// Defaults to the five most common platforms.
    list_platforms: Vec<String>,
}

impl CondaRegistryClient {
    pub fn new(base_url: impl Into<String>, opts: &UpstreamHttpOptions) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().user_agent("batlehub/0.1");
        let http = apply_upstream_options(builder, opts)?;
        Ok(Self {
            http,
            base_url: base_url.into(),
            basic_auth: opts.basic_auth.clone(),
            list_platforms: default_list_platforms(),
        })
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let rb = self.http.get(url);
        match &self.basic_auth {
            Some((u, p)) => rb.basic_auth(u, Some(p)),
            None => rb,
        }
    }

    /// Fetch one platform's `repodata.json` and return all versions of `package` found in it.
    /// Returns an empty `Vec` on any network/parse error (fail-open for version listing).
    async fn fetch_platform_versions(
        &self,
        base: &str,
        platform: &str,
        package: &str,
    ) -> Vec<String> {
        let url = format!("{base}/{platform}/repodata.json");
        let resp = match self.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            _ => return vec![],
        };
        let body = match resp.bytes().await {
            Ok(b) => b,
            Err(_) => return vec![],
        };
        let repodata: CondaRepodata = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        repodata
            .packages
            .values()
            .chain(repodata.packages_conda.values())
            .filter(|e| e.name.as_deref() == Some(package))
            .filter_map(|e| e.version.clone())
            .collect()
    }

    /// Look up a specific conda file in `{platform}/repodata.json`.
    async fn lookup_file_in_repodata(
        &self,
        base: &str,
        platform: &str,
        filename: &str,
        pkg: &PackageId,
    ) -> Result<PackageMetadata, CoreError> {
        let repodata_url = format!("{base}/{platform}/repodata.json");
        let resp = self
            .get(&repodata_url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("conda: repodata request failed: {e}")))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "conda repodata not found for platform '{platform}'"
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "conda upstream returned {} fetching repodata",
                resp.status()
            )));
        }
        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let body = resp
            .bytes()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;
        let repodata: CondaRepodata = serde_json::from_slice(&body)
            .map_err(|e| CoreError::Registry(format!("conda: parse repodata: {e}")))?;
        let entry = repodata
            .packages
            .get(filename)
            .or_else(|| repodata.packages_conda.get(filename));
        let entry = entry.ok_or_else(|| {
            CoreError::NotFound(format!(
                "conda: '{filename}' not found in {platform}/repodata.json"
            ))
        })?;
        let published_at = entry.timestamp.and_then(|ms| {
            chrono::DateTime::from_timestamp(ms / 1000, ((ms % 1000) * 1_000_000) as u32)
        });
        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at,
            download_url: Some(format!("{base}/{platform}/{filename}")),
            checksum: entry.sha256.clone(),
            is_signed: None,
            extra: serde_json::json!({
                "name": entry.name,
                "version": entry.version,
                "build": entry.build,
            }),
            cache_control,
        })
    }

    fn artifact_url(&self, pkg: &PackageId) -> String {
        let base = self.base_url.trim_end_matches('/');
        let platform = &pkg.version; // version = platform for conda

        match pkg.artifact.as_deref() {
            None | Some("repodata.json") => {
                format!("{base}/{platform}/repodata.json")
            }
            Some("current_repodata.json") => {
                format!("{base}/{platform}/current_repodata.json")
            }
            Some(filename) => {
                format!("{base}/{platform}/{filename}")
            }
        }
    }
}

/// The five platforms queried by `list_versions` to synthesise a version list
/// from `repodata.json`.  `noarch` covers pure-Python and architecture-neutral
/// packages and is tried first because it is the smallest repodata file.
fn default_list_platforms() -> Vec<String> {
    ["noarch", "linux-64", "osx-64", "osx-arm64", "win-64"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ── RegistryClient impl ───────────────────────────────────────────────────────

#[async_trait]
impl RegistryClient for CondaRegistryClient {
    fn registry_type(&self) -> &str {
        "conda"
    }

    async fn resolve_metadata(&self, pkg: &PackageId) -> Result<PackageMetadata, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let platform = &pkg.version;

        // For specific package files, look them up in repodata.json.
        if pkg.name != "repodata" {
            if let Some(filename) = &pkg.artifact {
                return self
                    .lookup_file_in_repodata(base, platform, filename, pkg)
                    .await;
            }
        }

        // For repodata.json itself, return the URL as the download URL.
        let url = self.artifact_url(pkg);
        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(format!("conda metadata request failed: {e}")))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "conda resource not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "conda upstream returned {} for {}",
                resp.status(),
                pkg.cache_key()
            )));
        }

        let cache_control = resp
            .headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        Ok(PackageMetadata {
            id: pkg.clone(),
            published_at: None,
            download_url: Some(url),
            checksum: None,
            is_signed: None,
            extra: serde_json::Value::Null,
            cache_control,
        })
    }

    async fn fetch_artifact(&self, pkg: &PackageId) -> Result<FetchedArtifact, CoreError> {
        let url = self.artifact_url(pkg);

        tracing::debug!(url = %url, "fetching conda artifact");

        let resp = self
            .get(&url)
            .send()
            .await
            .map_err(|e| CoreError::Registry(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "conda artifact not found: {}",
                pkg.cache_key()
            )));
        }
        if !resp.status().is_success() {
            return Err(CoreError::Registry(format!(
                "conda upstream returned {} for {}",
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

    /// Synthesise a version list by scanning `repodata.json` for each of the
    /// configured `list_platforms`.  Platforms that return a 404 or network
    /// error are silently skipped so a missing platform never blocks warming.
    /// Versions are collected into a sorted, deduplicated list.
    async fn list_versions(&self, package: &str) -> Result<Vec<String>, CoreError> {
        let base = self.base_url.trim_end_matches('/');
        let mut versions: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for platform in &self.list_platforms {
            let platform_versions = self.fetch_platform_versions(base, platform, package).await;
            versions.extend(platform_versions);
        }

        Ok(versions.into_iter().collect())
    }
}

// ── Serde types for repodata.json ─────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
struct CondaRepodata {
    #[serde(default)]
    packages: std::collections::HashMap<String, CondaPackageEntry>,
    #[serde(default, rename = "packages.conda")]
    packages_conda: std::collections::HashMap<String, CondaPackageEntry>,
}

#[derive(Debug, Deserialize)]
struct CondaPackageEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    build: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    /// Build timestamp in milliseconds since the Unix epoch.
    /// Present in most but not all repodata.json entries.
    #[serde(default)]
    timestamp: Option<i64>,
}

// ── Local publish helpers ─────────────────────────────────────────────────────

/// Metadata extracted from a conda package archive.
#[derive(Debug, Clone)]
pub struct CondaPackageInfo {
    pub name: String,
    pub version: String,
    pub build: String,
    pub build_number: u64,
    pub depends: Vec<String>,
    pub subdir: Option<String>,
    pub license: Option<String>,
}

/// Parse a conda package (`.tar.bz2` or `.conda`) and extract `info/index.json`.
///
/// Supports:
/// - `.tar.bz2`: bzip2-compressed tar archive directly containing `info/index.json`
/// - `.conda`: ZIP archive containing `info-*.tar.zst` (zstd-compressed tar with `info/index.json`)
#[cfg(feature = "local-registry")]
pub fn parse_conda_metadata(data: &[u8]) -> Result<CondaPackageInfo, CoreError> {
    if is_zip(data) {
        parse_conda_format(data)
    } else {
        parse_tar_bz2(data)
    }
}

fn is_zip(data: &[u8]) -> bool {
    data.len() >= 4 && &data[..4] == b"PK\x03\x04"
}

#[cfg(feature = "local-registry")]
fn parse_tar_bz2(data: &[u8]) -> Result<CondaPackageInfo, CoreError> {
    use bzip2::read::BzDecoder;
    use std::io::Cursor;

    let cursor = Cursor::new(data);
    let bz = BzDecoder::new(cursor);
    let mut archive = tar::Archive::new(bz);

    let index_bytes = find_in_tar(&mut archive, "info/index.json")
        .map_err(|e| CoreError::Registry(format!("conda: read .tar.bz2 archive: {e}")))?;

    parse_index_json(&index_bytes)
}

#[cfg(feature = "local-registry")]
fn parse_conda_format(data: &[u8]) -> Result<CondaPackageInfo, CoreError> {
    use std::io::{Cursor, Read};
    use zip::ZipArchive;

    let cursor = Cursor::new(data);
    let mut zip = ZipArchive::new(cursor)
        .map_err(|e| CoreError::Registry(format!("conda: open .conda ZIP: {e}")))?;

    // Find the info-*.tar.zst member (collect names first to avoid borrow issues)
    let mut info_entry_name: Option<String> = None;
    for i in 0..zip.len() {
        if let Ok(f) = zip.by_index(i) {
            let name = f.name().to_owned();
            if name.starts_with("info-") && name.ends_with(".tar.zst") {
                info_entry_name = Some(name);
                break;
            }
        }
    }
    let info_entry_name = info_entry_name
        .ok_or_else(|| CoreError::Registry("conda: info-*.tar.zst not found in .conda".into()))?;

    let mut entry = zip
        .by_name(&info_entry_name)
        .map_err(|e| CoreError::Registry(format!("conda: open {info_entry_name}: {e}")))?;

    let mut zst_bytes = Vec::new();
    entry
        .read_to_end(&mut zst_bytes)
        .map_err(|e| CoreError::Registry(format!("conda: read {info_entry_name}: {e}")))?;

    let decoder = zstd::Decoder::new(zst_bytes.as_slice())
        .map_err(|e| CoreError::Registry(format!("conda: zstd decoder: {e}")))?;
    let mut archive = tar::Archive::new(decoder);

    let index_bytes = find_in_tar(&mut archive, "info/index.json")
        .map_err(|e| CoreError::Registry(format!("conda: read info tar: {e}")))?;

    parse_index_json(&index_bytes)
}

#[cfg(feature = "local-registry")]
fn find_in_tar<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    target: &str,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let matches = entry
            .path()
            .map(|p| p.as_os_str() == target || p.to_str() == Some(target))
            .unwrap_or(false);
        if matches {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("{target} not found in archive"),
    ))
}

#[derive(Debug, Deserialize)]
struct CondaIndexJson {
    name: String,
    version: String,
    build: String,
    #[serde(default)]
    build_number: u64,
    #[serde(default)]
    depends: Vec<String>,
    #[serde(default)]
    subdir: Option<String>,
    #[serde(default)]
    license: Option<String>,
}

fn parse_index_json(bytes: &[u8]) -> Result<CondaPackageInfo, CoreError> {
    let idx: CondaIndexJson = serde_json::from_slice(bytes)
        .map_err(|e| CoreError::Registry(format!("conda: parse info/index.json: {e}")))?;
    Ok(CondaPackageInfo {
        name: idx.name,
        version: idx.version,
        build: idx.build,
        build_number: idx.build_number,
        depends: idx.depends,
        subdir: idx.subdir,
        license: idx.license,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolve_metadata_repodata_returns_download_url() {
        let mut server = mockito::Server::new_async().await;
        let repodata = serde_json::json!({
            "packages": {
                "numpy-1.26.0-py311h0.tar.bz2": {
                    "name": "numpy",
                    "version": "1.26.0",
                    "build": "py311h0",
                    "sha256": "deadbeef"
                }
            },
            "packages.conda": {}
        });
        let _mock = server
            .mock("GET", "/linux-64/repodata.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(repodata.to_string())
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();

        let pkg = PackageId::new("my-conda", "numpy-1.26.0-py311h0", "linux-64")
            .with_artifact("numpy-1.26.0-py311h0.tar.bz2");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert_eq!(meta.checksum.as_deref(), Some("deadbeef"));
        assert!(meta
            .download_url
            .as_deref()
            .unwrap()
            .ends_with("linux-64/numpy-1.26.0-py311h0.tar.bz2"));
    }

    #[tokio::test]
    async fn list_versions_aggregates_across_platforms() {
        let mut server = mockito::Server::new_async().await;

        // noarch: numpy 1.26.0
        let noarch = serde_json::json!({
            "packages": {
                "numpy-1.26.0-pyhd8ed1ab_0.tar.bz2": { "name": "numpy", "version": "1.26.0", "build": "pyhd8ed1ab_0" }
            },
            "packages.conda": {}
        });
        // linux-64: numpy 1.26.0 and 1.25.2 (a binary build + older version)
        let linux64 = serde_json::json!({
            "packages": {},
            "packages.conda": {
                "numpy-1.26.0-py311h0_0.conda": { "name": "numpy", "version": "1.26.0", "build": "py311h0_0" },
                "numpy-1.25.2-py311h0_0.conda": { "name": "numpy", "version": "1.25.2", "build": "py311h0_0" }
            }
        });
        let _m1 = server
            .mock("GET", "/noarch/repodata.json")
            .with_status(200)
            .with_body(noarch.to_string())
            .create_async()
            .await;
        let _m2 = server
            .mock("GET", "/linux-64/repodata.json")
            .with_status(200)
            .with_body(linux64.to_string())
            .create_async()
            .await;
        // Other platforms return 404 — should be silently skipped
        let _m3 = server
            .mock("GET", "/osx-64/repodata.json")
            .with_status(404)
            .create_async()
            .await;
        let _m4 = server
            .mock("GET", "/osx-arm64/repodata.json")
            .with_status(404)
            .create_async()
            .await;
        let _m5 = server
            .mock("GET", "/win-64/repodata.json")
            .with_status(404)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("numpy").await.unwrap();

        // Sorted, deduplicated: 1.25.2 before 1.26.0 (lexicographic)
        assert_eq!(versions, vec!["1.25.2", "1.26.0"]);
    }

    #[tokio::test]
    async fn list_versions_returns_empty_for_unknown_package() {
        let mut server = mockito::Server::new_async().await;
        let repodata = serde_json::json!({ "packages": {}, "packages.conda": {} });
        for platform in ["noarch", "linux-64", "osx-64", "osx-arm64", "win-64"] {
            server
                .mock("GET", &*format!("/{platform}/repodata.json"))
                .with_status(200)
                .with_body(repodata.to_string())
                .create_async()
                .await;
        }
        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();
        let versions = client.list_versions("nonexistent-pkg").await.unwrap();
        assert!(versions.is_empty());
    }

    #[tokio::test]
    async fn resolve_metadata_parses_timestamp() {
        let mut server = mockito::Server::new_async().await;
        // timestamp = 1697145600000 ms → 2023-10-12T20:00:00Z
        let repodata = serde_json::json!({
            "packages": {
                "numpy-1.26.0-py311h0.tar.bz2": {
                    "name": "numpy",
                    "version": "1.26.0",
                    "build": "py311h0",
                    "sha256": "abc",
                    "timestamp": 1697145600000i64
                }
            },
            "packages.conda": {}
        });
        let _mock = server
            .mock("GET", "/linux-64/repodata.json")
            .with_status(200)
            .with_body(repodata.to_string())
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = PackageId::new("reg", "numpy-1.26.0-py311h0", "linux-64")
            .with_artifact("numpy-1.26.0-py311h0.tar.bz2");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert!(
            meta.published_at.is_some(),
            "published_at should be parsed from timestamp"
        );
        let ts = meta.published_at.unwrap();
        assert_eq!(ts.timestamp(), 1697145600);
    }

    #[tokio::test]
    async fn resolve_metadata_no_timestamp_gives_none() {
        let mut server = mockito::Server::new_async().await;
        let repodata = serde_json::json!({
            "packages": {
                "bzip2-1.0.8-h5.tar.bz2": {
                    "name": "bzip2",
                    "version": "1.0.8",
                    "build": "h5",
                    "sha256": "def"
                }
            },
            "packages.conda": {}
        });
        let _mock = server
            .mock("GET", "/linux-64/repodata.json")
            .with_status(200)
            .with_body(repodata.to_string())
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = PackageId::new("reg", "bzip2-1.0.8-h5", "linux-64")
            .with_artifact("bzip2-1.0.8-h5.tar.bz2");
        let meta = client.resolve_metadata(&pkg).await.unwrap();
        assert!(
            meta.published_at.is_none(),
            "published_at should be None when timestamp is absent"
        );
    }

    #[tokio::test]
    async fn repodata_404_returns_not_found() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/linux-64/repodata.json")
            .with_status(404)
            .create_async()
            .await;

        let opts = UpstreamHttpOptions::default();
        let client = CondaRegistryClient::new(server.url(), &opts).unwrap();
        let pkg = PackageId::new("reg", "repodata", "linux-64");
        let err = client.resolve_metadata(&pkg).await.unwrap_err();
        assert!(matches!(err, CoreError::NotFound(_)));
    }

    #[cfg(feature = "local-registry")]
    #[test]
    fn parse_conda_metadata_tar_bz2() {
        use bzip2::write::BzEncoder;
        use bzip2::Compression;
        use std::io::Write;

        let index_json = serde_json::json!({
            "name": "test-pkg",
            "version": "1.0.0",
            "build": "py311h0_0",
            "build_number": 0,
            "depends": ["python >=3.11"],
            "subdir": "linux-64"
        });
        let index_bytes = serde_json::to_vec(&index_json).unwrap();

        // Build a tar archive
        let mut tar_bytes = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes);
            let mut header = tar::Header::new_gnu();
            header.set_size(index_bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder
                .append_data(&mut header, "info/index.json", index_bytes.as_slice())
                .unwrap();
            tar_builder.finish().unwrap();
        }

        // Compress with bzip2
        let mut encoder = BzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&tar_bytes).unwrap();
        let compressed = encoder.finish().unwrap();

        let info = parse_conda_metadata(&compressed).unwrap();
        assert_eq!(info.name, "test-pkg");
        assert_eq!(info.version, "1.0.0");
        assert_eq!(info.build, "py311h0_0");
        assert_eq!(info.depends, vec!["python >=3.11"]);
    }
}
