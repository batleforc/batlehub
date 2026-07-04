//! Publish handlers for locally-hosted Debian (`deb`) and RPM (`rpm`) repositories.
//!
//! Uploading a package stores it under a `local:` storage key, records a small
//! metadata sidecar, and regenerates the affected index files (APT
//! `Packages`/`Release`, RPM `repodata/`) — signing them with the registry's
//! Ed25519 OpenPGP key when one is configured. Index generation itself lives in
//! `batlehub_adapters::repo`; this module orchestrates storage I/O around it.

use std::sync::Arc;

use actix_web::{put, web, HttpResponse, Responder};

use batlehub_adapters::repo::{deb, gzip, pacman, rpm, OpenPgpSigner};
use batlehub_core::{
    ports::{StorageBackend, StorageMeta},
    services::{LocalRegistryService, PublishPolicyRequest},
};

use super::super::common::{collect_payload, collect_storage_stream, require_registry_type};
use super::repo_storage_key;
use crate::{
    error::AppError, extractors::AuthIdentity, handlers::back_office::require_authenticated,
    RegistryMap, RegistryModeMap, RepoSignerMap,
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

/// Read many sidecar keys concurrently (bounded), preserving input order and
/// dropping any that no longer exist. Index regeneration re-reads every sidecar
/// in the repo, so issuing those reads concurrently rather than one-at-a-time
/// keeps per-publish latency from growing linearly with the read round-trips.
async fn read_many(
    storage: &dyn StorageBackend,
    keys: Vec<String>,
) -> Result<Vec<Vec<u8>>, AppError> {
    use futures::stream::{StreamExt, TryStreamExt};
    const MAX_CONCURRENT_READS: usize = 16;
    futures::stream::iter(keys)
        .map(|k| async move { read_opt(storage, &k).await })
        .buffered(MAX_CONCURRENT_READS)
        .try_filter_map(|opt| async move { Ok(opt) })
        .try_collect()
        .await
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
    // `validate_path_safe` permits interior '/', but `arch` is a single path
    // segment in the sidecar key that `regenerate_deb` splits on — a '/' would
    // shift the (component, arch) grouping — so reject it explicitly.
    batlehub_core::services::validate_path_safe("architecture", &arch).map_err(AppError::from)?;
    if arch.contains('/') {
        return Err(AppError::bad_request(
            "architecture must not contain '/'".to_owned(),
        ));
    }

    // Enforce the shared publish policy (role >= User, versioning, signing,
    // namespace, ownership, configured size limit, and quota) before writing to
    // the repo layout — deb/rpm host their files outside the standard
    // package-version model, so they call this directly instead of `publish()`.
    let artifact_len = bytes.len() as u64;
    local_svc
        .enforce_publish_policy(
            &PublishPolicyRequest {
                registry: &registry,
                name: &name,
                version: &version,
                artifact_len,
                signature_bytes: None,
                signature_type: None,
            },
            &identity.0,
        )
        .await
        .map_err(AppError::from)?;

    // `enforce_publish_policy` has already recorded quota; revoke it if any storage
    // step below fails so a transient write error doesn't permanently charge the
    // publisher for an artifact that never landed (mirrors `publish()`'s rollback).
    let stored: Result<(), AppError> = async {
        let storage = local_svc.storage.as_ref();
        let pool = deb::pool_path(&component, &pkg).map_err(AppError::from)?;

        // 1. Store the .deb in the pool (`Bytes` clone is a cheap refcount bump).
        store_bytes(storage, &repo_storage_key(&registry, &pool), bytes.clone()).await?;

        // 2. Record the Packages stanza as a sidecar keyed by suite/component/arch.
        let stanza = deb::packages_stanza(&pkg, &pool);
        let sidecar = format!(
            "local:{registry}/_index/deb/{distribution}/{component}/{arch}/{name}_{version}.stanza"
        );
        store_bytes(storage, &sidecar, stanza.into_bytes()).await?;

        // 3. Regenerate the suite indexes (Packages per component/arch + Release).
        let signer = signers.get(&registry);
        regenerate_deb(storage, &registry, &distribution, signer.as_deref()).await
    }
    .await;
    if let Err(e) = stored {
        local_svc
            .revoke_publish_quota(&identity.0, &registry, artifact_len)
            .await;
        return Err(e);
    }

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
        // `read_many` preserves the (sorted) key order, so the generated Packages
        // file stays deterministic.
        let stanzas: Vec<String> = read_many(storage, sidecar_keys)
            .await?
            .iter()
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .collect();
        let packages = deb::generate_packages(&stanzas);
        let packages_bytes = packages.into_bytes();
        let gz = gzip(&packages_bytes).map_err(|e| AppError::internal(e.to_string()))?;

        let rel_dir = format!("{component}/binary-{arch}");
        let pkg_path = format!("{rel_dir}/Packages");
        let gz_path = format!("{rel_dir}/Packages.gz");
        let pkg_key = repo_storage_key(registry, &format!("dists/{suite}/{pkg_path}"));
        let gz_key = repo_storage_key(registry, &format!("dists/{suite}/{gz_path}"));

        // Compute the Release entries (which copy out only the size/hash digests)
        // before moving the buffers into storage, so the index blobs aren't cloned.
        release_files.push(deb::ReleaseFile::new(pkg_path, &packages_bytes));
        release_files.push(deb::ReleaseFile::new(gz_path, &gz));
        store_bytes(storage, &pkg_key, packages_bytes).await?;
        store_bytes(storage, &gz_key, gz).await?;
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

    // Enforce the shared publish policy before writing to the repo layout
    // (see the matching call in `deb_publish`).
    let artifact_len = bytes.len() as u64;
    local_svc
        .enforce_publish_policy(
            &PublishPolicyRequest {
                registry: &registry,
                name: &pkg.name,
                version: &pkg.version,
                artifact_len,
                signature_bytes: None,
                signature_type: None,
            },
            &identity.0,
        )
        .await
        .map_err(AppError::from)?;

    // Revoke the quota recorded above if any storage step fails (see `deb_publish`).
    let stored: Result<(), AppError> = async {
        let storage = local_svc.storage.as_ref();
        // 1. Store the .rpm (`Bytes` clone is a cheap refcount bump).
        store_bytes(
            storage,
            &repo_storage_key(&registry, &location),
            bytes.clone(),
        )
        .await?;
        // 2. Record a JSON sidecar for repodata regeneration.
        let sidecar = format!("local:{registry}/_index/rpm/{id}.json");
        let json = serde_json::to_vec(&pkg).map_err(|e| AppError::internal(e.to_string()))?;
        store_bytes(storage, &sidecar, json).await?;
        // 3. Regenerate repodata.
        let signer = signers.get(&registry);
        regenerate_rpm(storage, &registry, signer.as_deref()).await
    }
    .await;
    if let Err(e) = stored {
        local_svc
            .revoke_publish_quota(&identity.0, &registry, artifact_len)
            .await;
        return Err(e);
    }

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
    let mut packages: Vec<rpm::RpmPackage> = read_many(storage, keys)
        .await?
        .into_iter()
        .filter_map(|bytes| serde_json::from_slice::<rpm::RpmPackage>(&bytes).ok())
        .collect();
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
        // Build the repomd entry (digests only) before moving the gz into storage.
        repomd_entries.push(rpm::RepoMdData::new(kind, &href, &gz, plain, timestamp));
        store_bytes(storage, &repo_storage_key(registry, &href), gz).await?;
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

// ── Pacman publish ───────────────────────────────────────────────────────────

/// `PUT /proxy/{registry}/pacman/upload`
#[utoipa::path(
    put,
    path = "/proxy/{registry}/pacman/upload",
    tag = "proxy/pacman",
    params(("registry" = String, Path, description = "Registry name")),
    responses(
        (status = 201, description = "Package published; repo database regenerated"),
        (status = 400, description = "Invalid package"),
        (status = 403, description = "Authentication required"),
        (status = 404, description = "Unknown or non-local registry"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/proxy/{registry}/pacman/upload")]
pub async fn pacman_publish(
    path: web::Path<String>,
    payload: web::Payload,
    identity: AuthIdentity,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
    signers: web::Data<RepoSignerMap>,
) -> Result<impl Responder, AppError> {
    let registry = path.into_inner();
    require_registry_type(&registry, "pacman", &map)?;
    super::super::common::require_local_mode(&registry, &mode_map)?;
    require_authenticated(&identity)?;

    let bytes = collect_payload(payload).await?;

    // Parse once with a placeholder filename, then name the stored file from the
    // parsed coordinates + the compression magic — we never trust a client name.
    let mut pkg = pacman::parse_pacman(&bytes, "").map_err(AppError::from)?;
    let name = pkg
        .name()
        .ok_or_else(|| AppError::bad_request(".PKGINFO is missing pkgname".to_owned()))?
        .to_owned();
    let version = pkg
        .version()
        .ok_or_else(|| AppError::bad_request(".PKGINFO is missing pkgver".to_owned()))?
        .to_owned();
    let arch = pkg
        .arch()
        .ok_or_else(|| AppError::bad_request(".PKGINFO is missing arch".to_owned()))?
        .to_owned();
    batlehub_core::services::validate_coordinate(&name, &version, None).map_err(AppError::from)?;
    // `arch` is a single path segment in both the package key (`{arch}/{file}`)
    // and the sidecar key that `regenerate_pacman` groups on, so reject traversal
    // and any '/' for a clean 400 rather than relying on the storage guard.
    batlehub_core::services::validate_path_safe("architecture", &arch).map_err(AppError::from)?;
    if arch.contains('/') {
        return Err(AppError::bad_request(
            "architecture must not contain '/'".to_owned(),
        ));
    }
    let filename = format!("{name}-{version}-{arch}{}", pacman::download_suffix(&bytes));
    pkg.filename = filename.clone();

    // Enforce the shared publish policy before writing to the repo layout
    // (see the matching call in `deb_publish`).
    let artifact_len = bytes.len() as u64;
    local_svc
        .enforce_publish_policy(
            &PublishPolicyRequest {
                registry: &registry,
                name: &name,
                version: &version,
                artifact_len,
                signature_bytes: None,
                signature_type: None,
            },
            &identity.0,
        )
        .await
        .map_err(AppError::from)?;

    // Revoke the quota recorded above if any storage step fails (see `deb_publish`).
    let stored: Result<(), AppError> = async {
        let storage = local_svc.storage.as_ref();
        let signer = signers.get(&registry);
        let pkg_path = format!("{arch}/{filename}");

        // 1. Store the package (`Bytes` clone is a cheap refcount bump).
        store_bytes(
            storage,
            &repo_storage_key(&registry, &pkg_path),
            bytes.clone(),
        )
        .await?;

        // 2. When signing, emit the detached `.sig` next to the package and embed
        //    the same signature (base64) in the DB `%PGPSIG%` field.
        if let Some(signer) = signer.as_deref() {
            let sig = signer.detached_sign_binary(&bytes);
            store_bytes(
                storage,
                &repo_storage_key(&registry, &format!("{pkg_path}.sig")),
                sig.clone(),
            )
            .await?;
            use base64::Engine;
            pkg.pgpsig = Some(base64::engine::general_purpose::STANDARD.encode(&sig));
        }

        // 3. Record a JSON sidecar, keyed by arch, for DB regeneration.
        let sidecar = format!("local:{registry}/_index/pacman/{arch}/{filename}.json");
        let json = serde_json::to_vec(&pkg).map_err(|e| AppError::internal(e.to_string()))?;
        store_bytes(storage, &sidecar, json).await?;

        // 4. Regenerate the per-arch database.
        regenerate_pacman(storage, &registry, &arch, signer.as_deref()).await
    }
    .await;
    if let Err(e) = stored {
        local_svc
            .revoke_publish_quota(&identity.0, &registry, artifact_len)
            .await;
        return Err(e);
    }

    Ok(HttpResponse::Created().body(format!("published {name} {version} ({arch})")))
}

/// Rebuild the `{arch}/{registry}.db` (and `.files`) database from the stored
/// pacman sidecars, optionally signing it with `<repo>.db.sig`.
async fn regenerate_pacman(
    storage: &dyn StorageBackend,
    registry: &str,
    arch: &str,
    signer: Option<&OpenPgpSigner>,
) -> Result<(), AppError> {
    let prefix = format!("local:{registry}/_index/pacman/{arch}/");
    let keys = storage.list_keys(&prefix).await.map_err(AppError::from)?;
    let mut packages: Vec<pacman::PacmanPackage> = read_many(storage, keys)
        .await?
        .into_iter()
        .filter_map(|bytes| {
            serde_json::from_slice::<pacman::PacmanPackage>(&bytes)
                .map_err(|e| {
                    // A sidecar that no longer deserializes (corruption / schema
                    // skew) is skipped so one bad entry can't wedge the whole DB,
                    // but log it: the package silently vanishes from the index.
                    tracing::warn!("pacman: skipping unreadable sidecar in {registry}/{arch}: {e}");
                })
                .ok()
        })
        .collect();
    // Deterministic order keyed by the DB directory name (`<name>-<version>`).
    packages.sort_by_key(|p| pacman::db_dir_name(p).unwrap_or_default());

    let entries: Vec<(String, String)> = packages
        .iter()
        .filter_map(|p| Some((pacman::db_dir_name(p)?, pacman::desc_entry(p))))
        .collect();
    let db = pacman::generate_db(&entries).map_err(|e| AppError::internal(e.to_string()))?;

    // `<repo>.db`/`<repo>.files` are what `pacman -Sy` fetches; the `.tar.gz`
    // aliases match what `repo-add` writes, so tooling that expects either works.
    for name in [
        format!("{registry}.db"),
        format!("{registry}.db.tar.gz"),
        format!("{registry}.files"),
        format!("{registry}.files.tar.gz"),
    ] {
        store_bytes(
            storage,
            &repo_storage_key(registry, &format!("{arch}/{name}")),
            db.clone(),
        )
        .await?;
    }

    if let Some(signer) = signer {
        let sig = signer.detached_sign_binary(&db);
        for name in [
            format!("{registry}.db.sig"),
            format!("{registry}.files.sig"),
        ] {
            store_bytes(
                storage,
                &repo_storage_key(registry, &format!("{arch}/{name}")),
                sig.clone(),
            )
            .await?;
        }
        store_bytes(
            storage,
            &repo_storage_key(registry, "key.gpg"),
            signer.armored_public_key().into_bytes(),
        )
        .await?;
    }
    Ok(())
}
