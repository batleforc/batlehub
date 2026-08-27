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

/// A version coordinate that has been published and then deleted (RFC 0016 §4.4).
///
/// The coordinate is spent. `(registry, name, version)` may never be occupied by
/// different bytes, whether the version was released by hand or reclaimed by a
/// retention policy, so this row is what the publish path consults before it
/// accepts anything.
///
/// The row holds two things with two different lifetimes (RFC 0016 §4.5): the
/// **claim** — the coordinate itself, permanent, and the reason the row exists —
/// and the **detail** — checksum, publisher, index metadata, signature — which is
/// audit history and ages out under `tombstone_detail_for`. Every detail field is
/// therefore `Option`, and `detail_compacted_at` says which of the two reasons a
/// `None` has: never recorded, or stripped by compaction. An auditor reading a
/// bare row needs to be able to tell those apart.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Tombstone {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// When the version was deleted. Always present — this is what makes the row
    /// a tombstone rather than a live version.
    pub deleted_at: DateTime<Utc>,
    /// The identity that deleted it, or `None` for a deletion by a principal with
    /// no user id (an unauthenticated admin path, or a retention run).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<String>,
    /// When the detail columns below were stripped. `None` means they were not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_compacted_at: Option<DateTime<Utc>>,
    /// When the version was originally published.
    ///
    /// Not part of the detail that compaction strips: eight bytes do not
    /// accumulate, and "how long did this coordinate live" is the first question
    /// asked of a tombstone whose index metadata is already gone.
    pub published_at: DateTime<Utc>,
    /// Detail: who published it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_by: Option<String>,
    /// Detail: SHA-256 hex of the artifact bytes that used to be here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl Tombstone {
    /// Whether the detail columns have been stripped by a compaction run.
    ///
    /// Distinct from "the detail is absent": a tombstone written by a path that
    /// never had a checksum is not compacted, and re-running compaction over it
    /// must not claim it was.
    pub fn is_compacted(&self) -> bool {
        self.detail_compacted_at.is_some()
    }

    /// The message a publish onto this coordinate is refused with.
    ///
    /// Shared by every backend so a caller sees the same refusal whichever store
    /// is behind it, and worded to say the thing that is actually true: the
    /// version is not *published*, it is *spent*. "Already exists" would send a
    /// publisher looking for bytes that are not there.
    pub fn burned_coordinate_message(&self) -> String {
        format!(
            "{}@{} was published and deleted on {} in registry '{}'; a published \
             version coordinate is never reused — publish under a new version",
            self.name,
            self.version,
            self.deleted_at.format("%Y-%m-%d"),
            self.registry,
        )
    }
}

/// What one tombstone-compaction run did, or — under `dry_run` — would have done.
///
/// Compaction is destructive to history even though it is not destructive to the
/// invariant (RFC 0016 §4.5), so it reports like eviction does and is dry-runnable
/// for the same reason.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CompactionReport {
    /// Tombstones whose detail was stripped (or would have been, under `dry_run`).
    pub compacted: u64,
    /// Tombstones examined and left alone — already compacted, or inside the window.
    pub skipped: u64,
    /// True when nothing was written.
    pub dry_run: bool,
    /// The coordinates compacted, `"{name}@{version}"`, for the operator reading
    /// a dry run before turning it off.
    #[serde(default)]
    pub coordinates: Vec<String>,
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
