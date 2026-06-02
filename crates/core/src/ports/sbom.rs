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
#[derive(Debug, Clone)]
pub struct SbomDependency {
    pub name: String,
    pub version_req: Option<String>,
    pub ecosystem: String,
}

/// Extracts dependency information from a package archive.
/// Implementations live in `crates/adapters` where archive crates are available.
pub trait SbomExtractor: Send + Sync {
    /// Parse `data` (the raw artifact bytes) for the given `registry_type` and return
    /// the list of direct dependencies found in the embedded manifest, or an empty vec
    /// if the format is unrecognised or no manifest is present.
    fn extract(&self, data: &Bytes, registry_type: &str) -> Vec<SbomDependency>;
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
