#![cfg(feature = "storage-s3")]

//! Integration tests for `S3StorageBackend`.
//!
//! Requires a running S3-compatible service. Set `S3_TEST_ENDPOINT` to opt in:
//!
//!   task test:s3                                  # starts MinIO automatically
//!   S3_TEST_ENDPOINT=http://127.0.0.1:19000 \
//!     AWS_ACCESS_KEY_ID=minioadmin \
//!     AWS_SECRET_ACCESS_KEY=minioadmin \
//!     cargo test -p batlehub-adapters --features storage-s3 --test s3_storage

use aws_config::BehaviorVersion;
use aws_sdk_s3::{Client, config::Builder as S3ConfigBuilder};
use bytes::Bytes;
use futures::StreamExt;
use batlehub_adapters::storage::S3StorageBackend;
use batlehub_core::ports::{StorageBackend, StorageMeta, StoredArtifact};

const BUCKET: &str = "test-artifacts";
const REGION: &str = "us-east-1";

/// Returns the S3 test endpoint, or `None` to skip the test.
/// Credentials are taken from `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`.
fn s3_endpoint() -> Option<String> {
    std::env::var("S3_TEST_ENDPOINT").ok()
}

async fn make_backend(endpoint: &str) -> S3StorageBackend {
    let sdk_config = aws_config::defaults(BehaviorVersion::latest())
        .region(aws_config::Region::new(REGION))
        .endpoint_url(endpoint)
        .load()
        .await;

    let s3_cfg = S3ConfigBuilder::from(&sdk_config)
        .force_path_style(true)
        .build();

    let client = Client::from_conf(s3_cfg);

    // Idempotent — no-op if the bucket was created by a previous test run.
    let _ = client.create_bucket().bucket(BUCKET).send().await;

    S3StorageBackend::from_client(client, BUCKET.to_owned(), String::new())
}

async fn collect(artifact: StoredArtifact) -> Vec<u8> {
    let mut out = Vec::new();
    let mut stream = artifact.stream;
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn store_and_retrieve_round_trip() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;

    let data = Bytes::from_static(b"hello, s3");
    backend
        .store("key-rt", data.clone(), StorageMeta {
            content_type: Some("text/plain".to_owned()),
            size: Some(data.len() as u64),
            ..Default::default()
        })
        .await
        .unwrap();

    let artifact = backend.retrieve("key-rt").await.unwrap().expect("should exist");
    assert_eq!(collect(artifact).await, b"hello, s3");
}

#[tokio::test]
async fn retrieve_missing_key_returns_none() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;
    assert!(backend.retrieve("no-such-key").await.unwrap().is_none());
}

#[tokio::test]
async fn exists_before_and_after_store() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;

    assert!(!backend.exists("key-ex").await.unwrap());

    backend
        .store("key-ex", Bytes::from_static(b"data"), StorageMeta::default())
        .await
        .unwrap();

    assert!(backend.exists("key-ex").await.unwrap());
}

#[tokio::test]
async fn delete_removes_artifact() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;

    backend
        .store("key-del", Bytes::from_static(b"bye"), StorageMeta::default())
        .await
        .unwrap();

    backend.delete("key-del").await.unwrap();

    assert!(!backend.exists("key-del").await.unwrap());
}

#[tokio::test]
async fn delete_missing_key_is_ok() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;
    // NoSuchKey on delete must not propagate as an error.
    backend.delete("ghost-key").await.unwrap();
}

#[tokio::test]
async fn stat_by_prefix_counts_and_sums_sizes() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;

    for i in 0..3u8 {
        backend
            .store(
                &format!("artifact:npm/stat-pkg-{i}"),
                Bytes::from(vec![i; 100]),
                StorageMeta { size: Some(100), ..Default::default() },
            )
            .await
            .unwrap();
    }

    let (count, bytes) = backend.stat_by_prefix("artifact:npm/stat-pkg-").await.unwrap();
    assert_eq!(count, 3);
    assert_eq!(bytes, 300);
}

#[tokio::test]
async fn delete_by_prefix_removes_only_matching_keys() {
    let Some(ep) = s3_endpoint() else { return };
    let backend = make_backend(&ep).await;

    for i in 0..3u8 {
        backend
            .store(
                &format!("artifact:npm/del-pkg-{i}"),
                Bytes::from(vec![0u8; 10]),
                StorageMeta::default(),
            )
            .await
            .unwrap();
    }
    // A key under a different prefix that must survive.
    backend
        .store("artifact:cargo/del-pkg-0", Bytes::from_static(b"keep"), StorageMeta::default())
        .await
        .unwrap();

    let deleted = backend.delete_by_prefix("artifact:npm/del-pkg-").await.unwrap();
    assert_eq!(deleted, 3);

    let (remaining, _) = backend.stat_by_prefix("artifact:npm/del-pkg-").await.unwrap();
    assert_eq!(remaining, 0);

    assert!(backend.exists("artifact:cargo/del-pkg-0").await.unwrap());
}
