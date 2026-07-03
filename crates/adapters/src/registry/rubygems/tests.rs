use super::*;
use batlehub_core::ports::RegistryClient;
use client::{parse_gem_yaml, split_gem_stem};
use futures::TryStreamExt;
use mockito::Server;

fn client(url: &str) -> RubyGemsRegistryClient {
    RubyGemsRegistryClient::new(url, &Default::default()).unwrap()
}

#[test]
fn split_gem_stem_simple() {
    assert_eq!(split_gem_stem("rails-7.1.0"), Some(("rails", "7.1.0")));
}

#[test]
fn split_gem_stem_hyphenated_name() {
    assert_eq!(
        split_gem_stem("json-jwt-1.0.0"),
        Some(("json-jwt", "1.0.0"))
    );
}

#[test]
fn split_gem_stem_platform() {
    assert_eq!(
        split_gem_stem("nokogiri-1.10.0-x86_64-linux"),
        Some(("nokogiri", "1.10.0-x86_64-linux"))
    );
}

#[test]
fn split_gem_stem_no_version() {
    assert_eq!(split_gem_stem("rails"), None);
}

#[tokio::test]
async fn list_versions_ok() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/v1/versions/rails.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"[{"number":"7.1.0"},{"number":"7.0.8"},{"number":"6.1.7"}]"#)
        .create_async()
        .await;

    let c = client(&server.url());
    let versions = c.list_versions("rails").await.unwrap();
    // Should be reversed to oldest-first
    assert_eq!(versions, vec!["6.1.7", "7.0.8", "7.1.0"]);
}

#[tokio::test]
async fn list_versions_not_found() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/v1/versions/unknown-gem.json")
        .with_status(404)
        .create_async()
        .await;

    let c = client(&server.url());
    let versions = c.list_versions("unknown-gem").await.unwrap();
    assert!(versions.is_empty());
}

#[tokio::test]
async fn fetch_artifact_gem_download() {
    let body = b"fake-gem-bytes";
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/gems/rails-7.1.0.gem")
        .with_status(200)
        .with_header("content-type", "application/octet-stream")
        .with_body(body.as_slice())
        .create_async()
        .await;

    let c = client(&server.url());
    let pkg = PackageId::new("rg", "rails", "7.1.0").with_artifact("gem");
    let fetched = c.fetch_artifact(&pkg).await.unwrap();
    let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
    let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
    assert_eq!(content, body);
}

#[tokio::test]
async fn fetch_artifact_not_found() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/gems/missing-1.0.0.gem")
        .with_status(404)
        .create_async()
        .await;

    let c = client(&server.url());
    let pkg = PackageId::new("rg", "missing", "1.0.0").with_artifact("gem");
    assert!(matches!(
        c.fetch_artifact(&pkg).await,
        Err(CoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn fetch_artifact_index() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/specs.4.8.gz")
        .with_status(200)
        .with_body(b"gz-data".as_slice())
        .create_async()
        .await;

    let c = client(&server.url());
    let pkg = PackageId::new("rg", "_index", "specs");
    let fetched = c.fetch_artifact(&pkg).await.unwrap();
    let chunks: Vec<_> = fetched.stream.try_collect().await.unwrap();
    let content: Vec<u8> = chunks.into_iter().flat_map(|b| b.to_vec()).collect();
    assert_eq!(content, b"gz-data");
}

#[tokio::test]
async fn resolve_metadata_parses_gem_info() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/api/v1/gems/rails.json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_header("cache-control", "max-age=60")
        .with_body(
            r#"{"name":"rails","version":"7.1.0","created_at":"2023-10-04T12:00:00.000Z","sha":"abc123"}"#,
        )
        .create_async()
        .await;

    let c = client(&server.url());
    let pkg = PackageId::new("rg", "rails", "info");
    let meta = c.resolve_metadata(&pkg).await.unwrap();
    assert_eq!(meta.checksum.as_deref(), Some("abc123"));
    assert_eq!(meta.cache_control.as_deref(), Some("max-age=60"));
    assert!(meta.published_at.is_some());
}

#[cfg(feature = "local-registry")]
#[test]
fn parse_gem_yaml_basic() {
    let yaml = r#"--- !ruby/object:Gem::Specification
name: rails
version: !ruby/object:Gem::Version
  version: '7.1.0'
platform: ruby
authors:
- David Heinemeier Hansson
summary: Full-stack web application framework.
"#;
    let meta = parse_gem_yaml(yaml).unwrap();
    assert_eq!(meta.name, "rails");
    assert_eq!(meta.version, "7.1.0");
    assert_eq!(meta.platform, "ruby");
    assert_eq!(
        meta.summary.as_deref(),
        Some("Full-stack web application framework.")
    );
    assert_eq!(meta.authors, vec!["David Heinemeier Hansson"]);
}
