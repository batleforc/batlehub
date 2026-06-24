use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Visibility level for a locally published package.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Anyone, including anonymous users, may download.
    #[default]
    Public,
    /// Any authenticated user may download.
    Internal,
    /// Only members of the owning team group may download.
    Team,
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Visibility::Public => write!(f, "public"),
            Visibility::Internal => write!(f, "internal"),
            Visibility::Team => write!(f, "team"),
        }
    }
}

impl std::str::FromStr for Visibility {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(Self::Public),
            "internal" => Ok(Self::Internal),
            "team" => Ok(Self::Team),
            other => Err(format!("unknown visibility: '{other}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn from_str_all_variants() {
        assert_eq!(Visibility::from_str("public").unwrap(), Visibility::Public);
        assert_eq!(
            Visibility::from_str("internal").unwrap(),
            Visibility::Internal
        );
        assert_eq!(Visibility::from_str("team").unwrap(), Visibility::Team);
    }

    #[test]
    fn from_str_unknown_is_err() {
        assert!(Visibility::from_str("private").is_err());
        assert!(Visibility::from_str("").is_err());
        assert!(Visibility::from_str("Public").is_err());
    }

    #[test]
    fn display_roundtrip() {
        for v in [Visibility::Public, Visibility::Internal, Visibility::Team] {
            let s = v.to_string();
            let back = Visibility::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn default_is_public() {
        assert_eq!(Visibility::default(), Visibility::Public);
    }
}

/// A package published directly to this BatleHub instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedPackage {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// SHA-256 hex of the artifact bytes.
    pub checksum: String,
    pub yanked: bool,
    /// Flagged as deprecated. Stays listed and downloadable; carries an optional
    /// `deprecation_message`. For npm the message is mirrored into
    /// `index_metadata.deprecated` (npm's native field).
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecation_message: Option<String>,
    /// Hidden from registry-protocol listings/index but still downloadable by
    /// exact coordinate. Filtered in `load_visible_versions`.
    #[serde(default)]
    pub unlisted: bool,
    /// Registry-specific index line as opaque JSON.
    /// For Cargo: serialised `CargoIndexEntry`.
    pub index_metadata: serde_json::Value,
    pub published_at: DateTime<Utc>,
    pub published_by: Option<String>,
    /// Raw signature bytes from the `X-Artifact-Signature` header (base64-decoded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_bytes: Option<Vec<u8>>,
    /// Signature type from the `X-Signature-Type` header (e.g. `"pgp"`, `"ed25519"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_type: Option<String>,
    /// Download visibility for this package.
    #[serde(default)]
    pub visibility: Visibility,
}

/// One newline-delimited line in a Cargo sparse index file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoIndexEntry {
    pub name: String,
    pub vers: String,
    pub deps: Vec<CargoDep>,
    pub cksum: String,
    pub features: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features2: Option<serde_json::Value>,
    pub yanked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoDep {
    pub name: String,
    /// Version requirement string (e.g. `"^1.0"`).
    pub req: String,
    pub features: Vec<String>,
    pub optional: bool,
    pub default_features: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// `"normal"`, `"dev"`, or `"build"`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explicit_name_in_toml: Option<String>,
}
