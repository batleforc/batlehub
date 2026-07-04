mod eco_composer_conda_pypi;
mod eco_go;
mod eco_rubygems_maven_nuget;
mod eco_terraform;
mod lifecycle;
mod publish;
mod read;

pub use publish::PublishPolicyRequest;

use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;

use crate::{
    entities::{
        AccessAction, AccessEvent, AccessResult, Identity, PackageId, PublishedPackage, Role,
        SbomFormat, Visibility,
    },
    error::CoreError,
    ports::{
        LocalRegistryBackend, OwnershipPort, PackageRepository, StorageBackend, StorageMeta,
        TeamNamespacePort,
    },
    services::{
        explore_cache::ExploreCache,
        hot_config::{HotConfigLock, VersioningPolicy},
        quota::{QuotaCheck, QuotaService},
        sbom::{SbomPublishOptions, SbomService},
    },
};

// `VersioningPolicy` and `SigningConfig` are defined in hot_config and re-exported from services.

pub(super) fn validate_version(version: &str, policy: &VersioningPolicy) -> Result<(), CoreError> {
    if policy.enforce_semver {
        match semver::Version::parse(version) {
            Err(_) => {
                return Err(CoreError::InvalidVersion(format!(
                    "version '{version}' is not valid semver"
                )));
            }
            Ok(sv) if !policy.allow_prerelease && !sv.pre.is_empty() => {
                return Err(CoreError::InvalidVersion(format!(
                    "pre-release versions are not allowed (got '{version}')"
                )));
            }
            Ok(_) => {}
        }
    }
    if let Some(ref re) = policy.version_pattern {
        if !re.is_match(version) {
            return Err(CoreError::InvalidVersion(format!(
                "version '{version}' does not match required pattern '{}'",
                re.as_str()
            )));
        }
    }
    Ok(())
}

/// Reject a package coordinate component (name or version) that would let a
/// publish escape the storage root once interpolated into a storage key
/// (`{registry}/{name}/{version}`).
///
/// Package names legitimately contain `/` (npm scopes like `@scope/name`,
/// GitHub `owner/repo`), so interior `/` is allowed — but a `..` path segment,
/// an absolute path, a backslash, or a NUL byte are rejected. This runs
/// unconditionally at publish time, independent of the (optional) versioning
/// policy, and complements the storage-backend chokepoint that guards reads.
pub fn validate_path_safe(kind: &str, value: &str) -> Result<(), CoreError> {
    if value.is_empty() {
        return Err(CoreError::InvalidInput(format!("{kind} must not be empty")));
    }
    if value.contains('\0') || value.contains('\\') {
        return Err(CoreError::InvalidInput(format!(
            "{kind} '{value}' contains an illegal character"
        )));
    }
    if value.starts_with('/') || value.ends_with('/') {
        return Err(CoreError::InvalidInput(format!(
            "{kind} '{value}' must not start or end with '/'"
        )));
    }
    if value.split('/').any(|segment| segment == "..") {
        return Err(CoreError::InvalidInput(format!(
            "{kind} '{value}' contains a path-traversal segment"
        )));
    }
    Ok(())
}

/// Validate a package name is safe to use as a storage-key component.
/// Reusable by registry adapters that accept a package name from the request.
pub fn validate_package_name(name: &str) -> Result<(), CoreError> {
    validate_path_safe("package name", name)
}

/// Validate every user-controlled component of a package coordinate (`name`,
/// `version`, and optional sub-`artifact`) before it is interpolated into a
/// storage/cache key.
///
/// This is the edge counterpart to the storage-backend `ensure_safe_key`
/// chokepoint: the proxy read funnel (`ProxyService::handle`) and the local read
/// funnel (`LocalRegistryService::get_artifact`) call it so a traversal attempt is
/// rejected with a clean `400` for every registry — present and future —
/// regardless of whether the individual adapter validated its own input.
pub fn validate_coordinate(
    name: &str,
    version: &str,
    artifact: Option<&str>,
) -> Result<(), CoreError> {
    validate_path_safe("package name", name)?;
    validate_path_safe("version", version)?;
    if let Some(artifact) = artifact {
        validate_path_safe("artifact", artifact)?;
    }
    Ok(())
}

pub(super) async fn check_team_visibility(
    ns_port: &dyn TeamNamespacePort,
    registry: &str,
    package: &str,
    identity: &Identity,
) -> Result<(), CoreError> {
    match ns_port.find_namespace(registry, package).await? {
        Some(ns)
            if identity
                .groups
                .iter()
                .any(|g| g.replace(' ', "") == ns.group_id.replace(' ', "")) =>
        {
            Ok(())
        }
        Some(ns) => Err(CoreError::AccessDenied(format!(
            "package visibility is 'team'; must be a member of group '{}'",
            ns.group_id
        ))),
        // No claim found: deny everyone. Falling back to "any authenticated user"
        // would allow non-team members to read team-private packages whenever
        // the namespace claim is missing or has been deleted.
        None => Err(CoreError::AccessDenied(
            "package visibility is 'team' but no namespace claim is configured; access denied"
                .into(),
        )),
    }
}

/// Input to `LocalRegistryService::publish`.
pub struct PublishRequest {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// Raw artifact bytes.
    pub artifact: Bytes,
    /// SHA-256 hex of `artifact`, computed by the caller (handler layer).
    pub checksum: String,
    /// Ecosystem-specific index metadata serialised as JSON.
    /// Cargo: serialised `CargoIndexEntry` (with `cksum` already set).
    /// npm: version metadata from the publish payload (`dist.tarball` stripped).
    /// VSIX: `{"id": "pub.name", "version": "1.0.0"}`.
    pub index_metadata: serde_json::Value,
    /// Identity of the publishing user.
    pub publisher: Identity,
    /// Raw signature bytes decoded from `X-Artifact-Signature` header, if present.
    pub signature_bytes: Option<Vec<u8>>,
    /// Signature type from `X-Signature-Type` header, if present.
    pub signature_type: Option<String>,
}

/// Authoritative local-registry service: publish, yank, index, artifact retrieval.
pub struct LocalRegistryService {
    pub backend: Arc<dyn LocalRegistryBackend>,
    pub storage: Arc<dyn StorageBackend>,
    /// Hot-swappable state (versioning, signing, beta_channel, size limit).
    pub hot: HotConfigLock,
    /// Optional publish quota enforcement. When `None`, quotas are disabled.
    pub quota: Option<Arc<QuotaService>>,
    /// Optional per-package ownership enforcement. When `None`, ownership is not enforced.
    pub ownership: Option<Arc<dyn OwnershipPort>>,
    /// Optional team namespace enforcement. When `None`, namespace gating is disabled.
    pub team_namespace: Option<Arc<dyn TeamNamespacePort>>,
    /// Optional SBOM service; when `None`, SBOM generation is disabled globally.
    pub sbom: Option<Arc<SbomService>>,
    /// Optional explore cache; invalidated automatically on successful publish.
    pub explore_cache: Option<Arc<ExploreCache>>,
    /// Optional access-log sink for local/hybrid downloads. When `None`, local
    /// reads produce no audit trail (matches the pre-existing behavior); when
    /// set, `get_artifact` records the same `AccessEvent::allowed_download`/
    /// `denied_download` shape `ProxyService::handle` already records for the
    /// proxy-fallback path, so Local-mode registries aren't a silent audit gap.
    pub access_log: Option<Arc<dyn PackageRepository>>,
}

/// OS/architecture pair identifying a specific Terraform provider binary.
#[derive(Debug, Clone, Copy)]
pub struct TerraformPlatform<'a> {
    pub os: &'a str,
    pub arch: &'a str,
}

/// Stable storage key for a locally published artifact.
/// Distinct from the proxy `artifact:…` namespace to avoid collisions.
pub fn artifact_storage_key(registry: &str, name: &str, version: &str) -> String {
    format!("local:{}/{}/{}", registry, name, version)
}

/// Storage key for a non-POM Maven artifact (jar, checksum, etc.).
/// Multiple artifact files can coexist under the same version.
pub fn maven_artifact_storage_key(
    registry: &str,
    name: &str,
    version: &str,
    filename: &str,
) -> String {
    format!("local:{}/{}/{}/{}", registry, name, version, filename)
}

/// Storage key for a Terraform provider platform binary.
pub fn tf_provider_binary_storage_key(
    registry: &str,
    namespace: &str,
    ptype: &str,
    version: &str,
    os: &str,
    arch: &str,
) -> String {
    format!("local:{registry}/providers/{namespace}/{ptype}/{version}/{os}-{arch}")
}

#[cfg(test)]
mod tests;
