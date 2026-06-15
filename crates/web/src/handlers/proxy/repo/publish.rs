//! Publish handlers for locally-hosted Debian (`deb`) and RPM (`rpm`) repositories.
//!
//! Uploading a package stores it under a `local:` storage key, records a small
//! metadata sidecar, and regenerates the affected index files (APT
//! `Packages`/`Release`, RPM `repodata/`) — signing them with the registry's
//! Ed25519 OpenPGP key when one is configured. Index generation itself lives in
//! `batlehub_adapters::repo`; this module orchestrates storage I/O around it.

use std::sync::Arc;

use actix_web::{put, web, HttpResponse, Responder};

use batlehub_adapters::repo::{deb, gzip, rpm, OpenPgpSigner};
use batlehub_core::{
    ports::{StorageBackend, StorageMeta},
    services::LocalRegistryService,
};

use super::super::common::{collect_payload, collect_storage_stream, require_registry_type};
use super::repo_storage_key;
use crate::{
    error::AppError, extractors::AuthIdentity, RegistryMap, RegistryModeMap, RepoSignerMap,
};

// ── Small storage helpers ───────────────────────────────────────────────────

async fn store_bytes(
    storage: &dyn StorageBackend,
    key: &str,
    bytes: impl Into<actix_web::web::Bytes>,
) -> Result<(), AppError> {
    storage
        .store(key, bytes.into(), StorageMeta::default())
        .await
        .map_err(AppError::from)
}

async fn read_opt(storage: &dyn StorageBackend, key: &str) -> Result<Option<Vec<u8>>, AppError> {
    match storage.retrieve(key).await.map_err(AppError::from)? {
        Some(a) => Ok(Some(collect_storage_stream(a.stream).await?.to_vec())),
        None => Ok(None),
    }
}

/// Reject anonymous callers. Publishing to a local deb/rpm repo requires an
/// authenticated identity.
fn require_authenticated(identity: &AuthIdentity) -> Result<(), AppError> {
    use batlehub_core::entities::Role;
    if identity.0.role == Role::Anonymous {
        return Err(AppError::forbidden(
            "publishing requires authentication".to_owned(),
        ));
    }
    Ok(())
}

fn http_date() -> String {
    chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S UTC")
        .to_string()
}

// ── Debian publish ───────────────────────────────────────────────────────────

/// `PUT /proxy/{registry}/deb/pool/{distribution}/{component}/upload`
#[utoipa::path(
    put,
    path = "/proxy/{registry}/deb/pool/{distribution}/{component}/upload",
    tag = "proxy/deb",
    params(
        ("registry" = String, Path, description = "Registry name"),
        ("distribution" = String, Path, description = "Suite / distribution (e.g. stable)"),
        ("component" = String, Path, description = "Component (e.g. main)"),
    ),
    responses(
        (status = 201, description = "Package published; indexes regenerated"),
        (status = 400, description = "Invalid .deb"),
        (status = 403, description = "Authentication required"),
        (status = 404, description = "Unknown or non-local registry"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/deb/pool/{distribution}/{component}/upload")]
pub async fn deb_publish(
    path: web::Path<(String, String, String)>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    signers: web::Data<RepoSignerMap>,
) -> Result<impl Responder, AppError> {
    let (registry, distribution, component) = path.into_inner();
    require_registry_type(&registry, "deb", &map)?;
    super::super::common::require_local_mode(&registry, &mode_map)?;
    require_authenticated(&identity)?;
    for (kind, v) in [("distribution", &distribution), ("component", &component)] {
        batlehub_core::services::validate_path_safe(kind, v).map_err(AppError::from)?;
    }

    let bytes = collect_payload(payload).await?;
    let pkg = deb::parse_deb(&bytes).map_err(AppError::from)?;
    let name = pkg
        .name()
        .ok_or_else(|| AppError::bad_request("control is missing Package".to_owned()))?
        .to_owned();
    let version = pkg
        .version()
        .ok_or_else(|| AppError::bad_request("control is missing Version".to_owned()))?
        .to_owned();
    batlehub_core::services::validate_coordinate(&name, &version, None).map_err(AppError::from)?;
    let arch = pkg.architecture().unwrap_or("all").to_owned();
    // Edge validation: `arch` comes from the uploaded control file and flows into
    // the pool path and sidecar storage key, so reject traversal/separators here
    // for a clean 400 rather than relying on the storage backend's deeper guard.
    batlehub_core::services::validate_path_safe("architecture", &arch).map_err(AppError::from)?;

    let storage = local_svc.storage.as_ref();
    let pool = deb::pool_path(&component, &pkg).map_err(AppError::from)?;

    // 1. Store the .deb in the pool.
    store_bytes(storage, &repo_storage_key(&registry, &pool), bytes).await?;

    // 2. Record the Packages stanza as a sidecar keyed by suite/component/arch.
    let stanza = deb::packages_stanza(&pkg, &pool);
    let sidecar = format!(
        "local:{registry}/_index/deb/{distribution}/{component}/{arch}/{name}_{version}.stanza"
    );
    store_bytes(storage, &sidecar, stanza.into_bytes()).await?;

    // 3. Regenerate the suite indexes (Packages per component/arch + Release).
    let signer = signers.get(&registry);
    regenerate_deb(storage, &registry, &distribution, signer.as_deref()).await?;

    Ok(HttpResponse::Created().body(format!("published {name} {version} ({arch})")))
}

/// Rebuild every `Packages`/`Packages.gz` under `dists/{suite}/` from the stored
/// stanzas, then the `Release` (+ `InRelease`/`Release.gpg` when signing).
async fn regenerate_deb(
    storage: &dyn StorageBackend,
    registry: &str,
    suite: &str,
    signer: Option<&OpenPgpSigner>,
) -> Result<(), AppError> {
    let index_prefix = format!("local:{registry}/_index/deb/{suite}/");
    let keys = storage
        .list_keys(&index_prefix)
        .await
        .map_err(AppError::from)?;

    // Group sidecar keys by (component, arch).
    let mut groups: std::collections::BTreeMap<(String, String), Vec<String>> =
        std::collections::BTreeMap::new();
    for key in keys {
        // …/_index/deb/{suite}/{component}/{arch}/{file}.stanza
        let rest = match key.strip_prefix(&index_prefix) {
            Some(r) => r,
            None => continue,
        };
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 3 {
            continue;
        }
        groups
            .entry((parts[0].to_owned(), parts[1].to_owned()))
            .or_default()
            .push(key);
    }

    let mut release_files: Vec<deb::ReleaseFile> = Vec::new();
    let mut components: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut architectures: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for ((component, arch), mut sidecar_keys) in groups {
        sidecar_keys.sort();
        let mut stanzas = Vec::new();
        for k in sidecar_keys {
            if let Some(bytes) = read_opt(storage, &k).await? {
                stanzas.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        let packages = deb::generate_packages(&stanzas);
        let packages_bytes = packages.into_bytes();
        let gz = gzip(&packages_bytes).map_err(|e| AppError::internal(e.to_string()))?;

        let rel_dir = format!("{component}/binary-{arch}");
        let pkg_path = format!("{rel_dir}/Packages");
        let gz_path = format!("{rel_dir}/Packages.gz");
        store_bytes(
            storage,
            &repo_storage_key(registry, &format!("dists/{suite}/{pkg_path}")),
            packages_bytes.clone(),
        )
        .await?;
        store_bytes(
            storage,
            &repo_storage_key(registry, &format!("dists/{suite}/{gz_path}")),
            gz.clone(),
        )
        .await?;

        release_files.push(deb::ReleaseFile::new(pkg_path, &packages_bytes));
        release_files.push(deb::ReleaseFile::new(gz_path, &gz));
        components.insert(component);
        architectures.insert(arch);
    }

    let arch_vec: Vec<String> = architectures.into_iter().collect();
    let comp_vec: Vec<String> = components.into_iter().collect();
    let date = http_date();
    let meta = deb::ReleaseMeta {
        origin: "BatleHub",
        label: "BatleHub",
        suite,
        codename: suite,
        architectures: &arch_vec,
        components: &comp_vec,
        date: &date,
    };
    let release = deb::generate_release(&meta, &release_files);

    store_bytes(
        storage,
        &repo_storage_key(registry, &format!("dists/{suite}/Release")),
        release.clone().into_bytes(),
    )
    .await?;

    if let Some(signer) = signer {
        let inrelease = signer.clear_sign(&release);
        store_bytes(
            storage,
            &repo_storage_key(registry, &format!("dists/{suite}/InRelease")),
            inrelease.into_bytes(),
        )
        .await?;
        let detached = signer.detached_sign(release.as_bytes());
        store_bytes(
            storage,
            &repo_storage_key(registry, &format!("dists/{suite}/Release.gpg")),
            detached.into_bytes(),
        )
        .await?;
        store_bytes(
            storage,
            &repo_storage_key(registry, "key.gpg"),
            signer.armored_public_key().into_bytes(),
        )
        .await?;
    }
    Ok(())
}

// ── RPM publish ────────────────────────────────────────────────────────────

/// `PUT /proxy/{registry}/rpm/upload`
#[utoipa::path(
    put,
    path = "/proxy/{registry}/rpm/upload",
    tag = "proxy/rpm",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 201, description = "Package published; repodata regenerated"),
        (status = 400, description = "Invalid .rpm"),
        (status = 403, description = "Authentication required"),
        (status = 404, description = "Unknown or non-local registry"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/rpm/upload")]
pub async fn rpm_publish(
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    signers: web::Data<RepoSignerMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "rpm", &map)?;
    super::super::common::require_local_mode(&registry, &mode_map)?;
    require_authenticated(&identity)?;

    let bytes = collect_payload(payload).await?;

    // Parse once; `location` is just a settable field, so there's no need to
    // re-parse (and re-hash) the whole artifact a second time.
    let mut pkg = rpm::parse_rpm(&bytes, "").map_err(AppError::from)?;
    // Reject a header missing its identifying fields: an empty name/version/
    // release/arch would otherwise yield a degenerate id and corrupt repodata.
    if pkg.name.is_empty()
        || pkg.version.is_empty()
        || pkg.release.is_empty()
        || pkg.arch.is_empty()
    {
        return Err(AppError::bad_request(
            "rpm header is missing name/version/release/arch".to_owned(),
        ));
    }
    // Epoch-aware id so two builds differing only in Epoch don't overwrite.
    let id = pkg.storage_id();
    batlehub_core::services::validate_path_safe("package", &id).map_err(AppError::from)?;
    let location = format!("packages/{id}.rpm");
    pkg.location = location.clone();

    let storage = local_svc.storage.as_ref();
    // 1. Store the .rpm.
    store_bytes(storage, &repo_storage_key(&registry, &location), bytes).await?;
    // 2. Record a JSON sidecar for repodata regeneration.
    let sidecar = format!("local:{registry}/_index/rpm/{id}.json");
    let json = serde_json::to_vec(&pkg).map_err(|e| AppError::internal(e.to_string()))?;
    store_bytes(storage, &sidecar, json).await?;
    // 3. Regenerate repodata.
    let signer = signers.get(&registry);
    regenerate_rpm(storage, &registry, signer.as_deref()).await?;

    Ok(HttpResponse::Created().body(format!("published {}", pkg.nevra())))
}

/// Rebuild `repodata/` (primary/filelists/other + repomd, optionally signed) from
/// the stored RPM sidecars.
async fn regenerate_rpm(
    storage: &dyn StorageBackend,
    registry: &str,
    signer: Option<&OpenPgpSigner>,
) -> Result<(), AppError> {
    let prefix = format!("local:{registry}/_index/rpm/");
    let keys = storage.list_keys(&prefix).await.map_err(AppError::from)?;
    let mut packages: Vec<rpm::RpmPackage> = Vec::new();
    for k in keys {
        if let Some(bytes) = read_opt(storage, &k).await? {
            if let Ok(p) = serde_json::from_slice::<rpm::RpmPackage>(&bytes) {
                packages.push(p);
            }
        }
    }
    packages.sort_by_key(|a| a.nevra());

    let primary = rpm::primary_xml(&packages).into_bytes();
    let filelists = rpm::filelists_xml(&packages).into_bytes();
    let other = rpm::other_xml(&packages).into_bytes();
    let timestamp = chrono::Utc::now().timestamp().max(0) as u64;

    let mut repomd_entries = Vec::new();
    for (kind, plain) in [
        ("primary", &primary),
        ("filelists", &filelists),
        ("other", &other),
    ] {
        let gz = gzip(plain).map_err(|e| AppError::internal(e.to_string()))?;
        let href = format!("repodata/{kind}.xml.gz");
        store_bytes(storage, &repo_storage_key(registry, &href), gz.clone()).await?;
        repomd_entries.push(rpm::RepoMdData::new(kind, &href, &gz, plain, timestamp));
    }

    let repomd = rpm::repomd_xml(&repomd_entries).into_bytes();
    store_bytes(
        storage,
        &repo_storage_key(registry, "repodata/repomd.xml"),
        repomd.clone(),
    )
    .await?;

    if let Some(signer) = signer {
        let asc = signer.detached_sign(&repomd);
        store_bytes(
            storage,
            &repo_storage_key(registry, "repodata/repomd.xml.asc"),
            asc.into_bytes(),
        )
        .await?;
        store_bytes(
            storage,
            &repo_storage_key(registry, "repodata/repomd.xml.key"),
            signer.armored_public_key().into_bytes(),
        )
        .await?;
    }
    Ok(())
}
