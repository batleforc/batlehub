use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SbomFormat {
    Spdx,
    CycloneDx,
}

impl SbomFormat {
    pub fn spec_version(&self) -> &'static str {
        match self {
            SbomFormat::Spdx => "2.3",
            SbomFormat::CycloneDx => "1.4",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SbomFormat::Spdx => "spdx",
            SbomFormat::CycloneDx => "cyclonedx",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "spdx" => Some(SbomFormat::Spdx),
            "cyclonedx" => Some(SbomFormat::CycloneDx),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SbomSource {
    Generated,
    Upstream,
    Extracted,
}

impl SbomSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SbomSource::Generated => "generated",
            SbomSource::Upstream => "upstream",
            SbomSource::Extracted => "extracted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "generated" => Some(SbomSource::Generated),
            "upstream" => Some(SbomSource::Upstream),
            "extracted" => Some(SbomSource::Extracted),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSbom {
    pub id: Uuid,
    pub artifact_key: String,
    pub registry: String,
    pub package_name: String,
    pub version: String,
    pub format: SbomFormat,
    pub spec_version: String,
    pub document: serde_json::Value,
    pub source: SbomSource,
    pub created_at: DateTime<Utc>,
    /// The licence the package's own manifest declared, verbatim.
    ///
    /// A column rather than a read into `document`, because `LicenseGateRule`
    /// queries it on the request path and `document` is an opaque blob whose
    /// shape differs per format — and, for `SbomSource::Upstream`, is a
    /// document BatleHub did not write. `None` means unknown: either the
    /// registry type has no manifest parser, or the manifest declared nothing.
    /// It never means "no licence" (RFC 0004-bis §13.1).
    pub license: Option<String>,
}
