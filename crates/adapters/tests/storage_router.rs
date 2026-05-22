//! Integration tests for `StorageRouter`.
//!
//! Requires a running PostgreSQL instance. Set `DATABASE_URL` to opt in:
//!
//!   task test:pg-cache                              # starts Postgres via Podman automatically
//!   DATABASE_URL=postgresql://batlehub:changeme@localhost/batlehub \
//!     cargo test -p batlehub-adapters --test storage_router

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use sqlx::PgPool;

use batlehub_adapters::storage::{FilesystemStorageBackend, StorageRouter};
use batlehub_core::ports::{StorageBackend, StorageMeta, StoredArtifact};

fn db_url() -> Option<String> {
    std::env::var("DATABASE_URL").ok()
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

// Returns a unique key safe to use across parallel tests.
fn ukey(registry: &str, name: &str) -> String {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("artifact:{registry}/t{id}-{name}")
}

async fn make_fs(label: &str) -> Arc<FilesystemStorageBackend> {
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("batlehub-router-{label}-{pid}-{id}"));
    Arc::new(FilesystemStorageBackend::new(dir).await.unwrap())
}

async fn pool(url: &str) -> PgPool {
    let p = PgPool::connect(url).await.expect("connect to postgres");
    batlehub_adapters::migrations::embedded_migrator().run(&p).await.expect("run migrations");
    p
}

async fn collect(artifact: StoredArtifact) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut stream = artifact.stream;
    while let Some(chunk) = stream.next().await {
        buf.extend_from_slice(&chunk.unwrap());
    }
    buf
}

fn single_backend_router(fs: Arc<FilesystemStorageBackend>, pool: PgPool) -> StorageRouter {
    let mut backends: HashMap<String, Arc<dyn StorageBackend>> = HashMap::new();
    backends.insert("default".to_owned(), fs);
    StorageRouter::new(backends, "default".to_owned(), HashMap::new(), pool)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn store_and_retrieve_round_trip() {
    let Some(url) = db_url() else { return };
    let router = single_backend_router(make_fs("rr").await, pool(&url).await);
    let key = ukey("npm", "test-pkg");

    router.store(&key, Bytes::from_static(b"hello, router"), StorageMeta::default()).await.unwrap();
    let artifact = router.retrieve(&key).await.unwrap().expect("should be found");
    assert_eq!(collect(artifact).await, b"hello, router");
}

#[tokio::test]
async fn retrieve_missing_key_returns_none() {
    let Some(url) = db_url() else { return };
    let router = single_backend_router(make_fs("miss").await, pool(&url).await);
    let key = ukey("npm", "no-such-pkg");
    assert!(router.retrieve(&key).await.unwrap().is_none());
}

#[tokio::test]
async fn exists_before_and_after_store() {
    let Some(url) = db_url() else { return };
    let router = single_backend_router(make_fs("ex").await, pool(&url).await);
    let key = ukey("npm", "ex-pkg");

    assert!(!router.exists(&key).await.unwrap());
    router.store(&key, Bytes::from_static(b"data"), StorageMeta::default()).await.unwrap();
    assert!(router.exists(&key).await.unwrap());
}

#[tokio::test]
async fn delete_removes_artifact() {
    let Some(url) = db_url() else { return };
    let router = single_backend_router(make_fs("del").await, pool(&url).await);
    let key = ukey("npm", "del-pkg");

    router.store(&key, Bytes::from_static(b"bye"), StorageMeta::default()).await.unwrap();
    router.delete(&key).await.unwrap();
    assert!(!router.exists(&key).await.unwrap());
}

#[tokio::test]
async fn delete_missing_key_is_ok() {
    let Some(url) = db_url() else { return };
    let router = single_backend_router(make_fs("ghost").await, pool(&url).await);
    let key = ukey("npm", "ghost");
    router.delete(&key).await.unwrap();
}

#[tokio::test]
async fn retrieve_falls_back_to_default_when_no_artifact_storage_record() {
    let Some(url) = db_url() else { return };
    let fs = make_fs("fallback").await;
    let key = ukey("npm", "direct-pkg");

    // Write directly to the FS backend — no artifact_storage record created.
    fs.store(&key, Bytes::from_static(b"direct"), StorageMeta::default()).await.unwrap();

    let router = single_backend_router(fs, pool(&url).await);
    // Router must fall back to resolve_backend_for_key and find the file.
    let artifact = router.retrieve(&key).await.unwrap().expect("fallback must succeed");
    assert_eq!(collect(artifact).await, b"direct");
}

#[tokio::test]
async fn routes_to_correct_named_backend() {
    let Some(url) = db_url() else { return };
    let fs_a = make_fs("named-a").await;
    let fs_b = make_fs("named-b").await;

    let npm_key = ukey("npm", "pkg");
    let cargo_key = ukey("cargo", "crate");

    let mut backends: HashMap<String, Arc<dyn StorageBackend>> = HashMap::new();
    backends.insert("backend-a".to_owned(), fs_a.clone());
    backends.insert("backend-b".to_owned(), fs_b.clone());

    let mut assignments = HashMap::new();
    assignments.insert("npm".to_owned(), "backend-a".to_owned());
    assignments.insert("cargo".to_owned(), "backend-b".to_owned());

    let router = StorageRouter::new(backends, "backend-a".to_owned(), assignments, pool(&url).await);

    router.store(&npm_key, Bytes::from_static(b"npm-data"), StorageMeta::default()).await.unwrap();
    router.store(&cargo_key, Bytes::from_static(b"cargo-data"), StorageMeta::default()).await.unwrap();

    assert!(fs_a.exists(&npm_key).await.unwrap(), "npm artifact must land in backend-a");
    assert!(!fs_b.exists(&npm_key).await.unwrap(), "npm artifact must NOT be in backend-b");
    assert!(fs_b.exists(&cargo_key).await.unwrap(), "cargo artifact must land in backend-b");
    assert!(!fs_a.exists(&cargo_key).await.unwrap(), "cargo artifact must NOT be in backend-a");
}

#[tokio::test]
async fn unknown_registry_uses_default_backend() {
    let Some(url) = db_url() else { return };
    let fs_default = make_fs("unk-default").await;
    let fs_other = make_fs("unk-other").await;

    let key = ukey("pypi", "some-package");

    let mut backends: HashMap<String, Arc<dyn StorageBackend>> = HashMap::new();
    backends.insert("default".to_owned(), fs_default.clone());
    backends.insert("other".to_owned(), fs_other.clone());

    // Only "npm" is assigned; "pypi" has no assignment → must fall back to default.
    let mut assignments = HashMap::new();
    assignments.insert("npm".to_owned(), "other".to_owned());

    let router = StorageRouter::new(backends, "default".to_owned(), assignments, pool(&url).await);
    router.store(&key, Bytes::from_static(b"pypi-data"), StorageMeta::default()).await.unwrap();

    assert!(fs_default.exists(&key).await.unwrap(), "unassigned registry must use default backend");
    assert!(!fs_other.exists(&key).await.unwrap());
}
