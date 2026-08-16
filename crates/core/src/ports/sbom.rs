use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, Utc};

use crate::entities::{ArtifactSbom, SbomFormat};
use crate::error::CoreError;

#[async_trait]
pub trait SbomRepository: Send + Sync {
    /// Store or replace an SBOM for the given artifact key and format (upsert).
    async fn upsert_sbom(&self, sbom: ArtifactSbom) -> Result<(), CoreError>;

    /// Fetch the SBOM for a specific artifact key and format.
    async fn get_sbom(
        &self,
        artifact_key: &str,
        format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError>;

    /// Fetch the most recently recorded SBOM for a registry/package/version,
    /// regardless of the exact `artifact_key` (proxy keys carry a per-registry
    /// artifact suffix such as `/tarball` or `/dl` that callers cannot predict).
    async fn get_sbom_by_coordinates(
        &self,
        registry: &str,
        package_name: &str,
        version: &str,
        format: &SbomFormat,
    ) -> Result<Option<ArtifactSbom>, CoreError>;

    /// The licence recorded for a registry/package/version, if one is known.
    ///
    /// Separate from [`Self::get_sbom_by_coordinates`] because
    /// `LicenseGateRule` runs on every gated request and needs one string, not
    /// a whole SBOM document — and because it must not have to pick a
    /// `SbomFormat` to ask a question that has nothing to do with format.
    ///
    /// `Ok(None)` means unknown, which is not the same as unlicensed.
    async fn get_license_for_coordinate(
        &self,
        registry: &str,
        package_name: &str,
        version: &str,
    ) -> Result<Option<String>, CoreError>;

    /// List SBOMs for org-level export, optionally filtered by registry and time range.
    async fn list_sboms_for_export(
        &self,
        registry: Option<&str>,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<ArtifactSbom>, CoreError>;
}

/// A single dependency parsed from a package manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SbomDependency {
    pub name: String,
    pub version_req: Option<String>,
    pub ecosystem: String,
}

/// What a package's own manifest declares, read in a single pass.
///
/// Dependencies and the licence come from the same file — `Cargo.toml`,
/// `package.json`, `pom.xml`, `METADATA`, `.nuspec` — so they are returned
/// together rather than through two trait methods that would each open and
/// decompress the archive (RFC 0004-bis §13.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedManifest {
    pub dependencies: Vec<SbomDependency>,
    /// The licence exactly as the manifest declares it, trimmed and no more.
    ///
    /// Deliberately not normalised to canonical SPDX here: `LicenseGateRule`
    /// does its own case-insensitive comparison, and rewriting `Apache License
    /// 2.0` to `Apache-2.0` at this layer would put a guess in the stored
    /// record where the operator can no longer see what the package actually
    /// said. `None` means the manifest declared nothing, which the gate treats
    /// as unknown rather than as permissive.
    pub license: Option<String>,
}

impl ExtractedManifest {
    /// The pre-licence shape: dependencies only, nothing declared.
    pub fn from_dependencies(dependencies: Vec<SbomDependency>) -> Self {
        Self {
            dependencies,
            license: None,
        }
    }
}

/// Registry types whose archives carry a manifest the extractor can read a
/// licence out of.
///
/// Declared here rather than in the adapter because two places need it and they
/// must not disagree: `ArchiveSbomExtractor`'s dispatch, and the config warning
/// that tells an operator their `license_gate` can never observe a licence on
/// this registry (RFC 0004-bis §13.1). A parser added to the adapter without
/// updating this list would leave operators warned about a type that now works;
/// a type added here without a parser would silence a warning that is still
/// true. `extractor/mod.rs` has the test that refuses the drift.
pub const LICENSE_EXTRACTION_TYPES: &[&str] = &["cargo", "maven", "npm", "nuget", "pypi"];

/// Extracts dependency and licence information from a package archive.
/// Implementations live in `crates/adapters` where archive crates are available.
pub trait SbomExtractor: Send + Sync {
    /// Parse `data` (the raw artifact bytes) for the given `registry_type` and return
    /// what the embedded manifest declares, or a default (no dependencies, no
    /// licence) if the format is unrecognised or no manifest is present.
    fn extract(&self, data: &Bytes, registry_type: &str) -> ExtractedManifest;
}

/// Fetches an SBOM document from an upstream registry API.
/// Implementations live in `crates/adapters` where reqwest is available.
#[async_trait]
pub trait UpstreamSbomFetcher: Send + Sync {
    /// Attempt to fetch a pre-built SBOM document from the upstream.
    /// Returns `None` if the upstream does not provide one.
    async fn fetch(
        &self,
        registry_type: &str,
        name: &str,
        version: &str,
    ) -> Result<Option<serde_json::Value>, CoreError>;
}
