use super::*;
use client::{rewrite_simple_html, rewrite_simple_json};

#[test]
fn name_normalization() {
    assert_eq!(normalize_name("Pillow"), "pillow");
    assert_eq!(normalize_name("my_pkg"), "my-pkg");
    assert_eq!(normalize_name("A.B.C"), "a-b-c");
    assert_eq!(normalize_name("My--Package"), "my-package");
    assert_eq!(normalize_name("requests"), "requests");
}

#[test]
fn rewrite_simple_html_rewrites_cdn_urls() {
    let html = br#"<a href="https://files.pythonhosted.org/packages/ab/cd/requests-2.28.0.tar.gz#sha256=abc">requests-2.28.0.tar.gz</a>"#;
    let out = rewrite_simple_html(html, "my-pypi", "http://localhost:8080");
    let out_str = std::str::from_utf8(&out).unwrap();
    assert!(out_str.contains("/proxy/my-pypi/packages/requests-2.28.0.tar.gz#sha256=abc"));
    assert!(!out_str.contains("files.pythonhosted.org"));
}

#[test]
fn rewrite_simple_html_keeps_relative_hrefs() {
    let html = br#"<a href="/simple/">index</a>"#;
    let out = rewrite_simple_html(html, "my-pypi", "http://localhost:8080");
    let out_str = std::str::from_utf8(&out).unwrap();
    assert!(out_str.contains(r#"href="/simple/""#));
}

#[test]
fn rewrite_simple_json_rewrites_urls() {
    let json = serde_json::json!({
        "files": [
            { "filename": "foo-1.0.whl", "url": "https://files.pythonhosted.org/packages/xx/foo-1.0.whl#sha256=deadbeef" }
        ]
    });
    let body = serde_json::to_vec(&json).unwrap();
    let out = rewrite_simple_json(&body, "my-pypi", "http://localhost");
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let url = parsed["files"][0]["url"].as_str().unwrap();
    assert_eq!(
        url,
        "http://localhost/proxy/my-pypi/packages/foo-1.0.whl#sha256=deadbeef"
    );
}

#[tokio::test]
async fn resolve_metadata_finds_wheel() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/pypi/requests/2.28.0/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&serde_json::json!({
            "urls": [
                {
                    "filename": "requests-2.28.0-py3-none-any.whl",
                    "url": "https://files.pythonhosted.org/packages/requests-2.28.0-py3-none-any.whl",
                    "digests": { "sha256": "abc123" },
                    "upload_time_iso_8601": "2022-10-26T18:17:01.491020Z"
                }
            ]
        })).unwrap())
        .create_async()
        .await;

    let opts = UpstreamHttpOptions::default();
    let client = PypiRegistryClient::new(server.url(), &opts).unwrap();

    let pkg = PackageId::new("my-pypi", "requests", "2.28.0")
        .with_artifact("requests-2.28.0-py3-none-any.whl");
    let meta = client.resolve_metadata(&pkg).await.unwrap();

    assert_eq!(meta.checksum.as_deref(), Some("abc123"));
    assert!(meta.download_url.is_some());
    assert!(meta.published_at.is_some());
    mock.assert_async().await;
}

#[tokio::test]
async fn resolve_metadata_404_returns_not_found() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/pypi/nonexistent/1.0.0/json")
        .with_status(404)
        .create_async()
        .await;

    let opts = UpstreamHttpOptions::default();
    let client = PypiRegistryClient::new(server.url(), &opts).unwrap();
    let pkg = PackageId::new("reg", "nonexistent", "1.0.0");
    let err = client.resolve_metadata(&pkg).await.unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn list_versions_parses_releases() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/pypi/requests/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::to_string(&serde_json::json!({
                "releases": {
                    "2.27.0": [],
                    "2.28.0": [],
                    "2.28.1": []
                }
            }))
            .unwrap(),
        )
        .create_async()
        .await;

    let opts = UpstreamHttpOptions::default();
    let client = PypiRegistryClient::new(server.url(), &opts).unwrap();
    let versions = client.list_versions("requests").await.unwrap();
    assert_eq!(versions, vec!["2.27.0", "2.28.0", "2.28.1"]);
}

// ── fetch_simple_page ──────────────────────────────────────────────────────

#[tokio::test]
async fn fetch_simple_page_returns_body_and_content_type() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/simple/my-package/")
        .with_status(200)
        .with_header("content-type", "text/html")
        .with_body("<html><body>index</body></html>")
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let (body, content_type) = fetch_simple_page(&client, &server.url(), "My_Package", None, None)
        .await
        .unwrap();

    assert_eq!(&body[..], b"<html><body>index</body></html>");
    assert_eq!(content_type.as_deref(), Some("text/html"));
    mock.assert_async().await;
}

#[tokio::test]
async fn fetch_simple_page_404_returns_not_found() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/simple/nonexistent/")
        .with_status(404)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let err = fetch_simple_page(&client, &server.url(), "nonexistent", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::NotFound(_)));
}

#[tokio::test]
async fn fetch_simple_page_500_returns_registry_error() {
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("GET", "/simple/broken/")
        .with_status(500)
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let err = fetch_simple_page(&client, &server.url(), "broken", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, CoreError::Registry(_)));
}

#[tokio::test]
async fn fetch_simple_page_sends_basic_auth_and_accept_headers() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/simple/private-pkg/")
        .match_header("authorization", mockito::Matcher::Regex("^Basic .*".into()))
        .match_header("accept", "application/vnd.pypi.simple.v1+json")
        .with_status(200)
        .with_header("content-type", "application/vnd.pypi.simple.v1+json")
        .with_body("{}")
        .create_async()
        .await;

    let client = reqwest::Client::new();
    let auth = ("user".to_owned(), "pass".to_owned());
    let (_body, content_type) = fetch_simple_page(
        &client,
        &server.url(),
        "private-pkg",
        Some(&auth),
        Some("application/vnd.pypi.simple.v1+json"),
    )
    .await
    .unwrap();

    assert_eq!(
        content_type.as_deref(),
        Some("application/vnd.pypi.simple.v1+json")
    );
    mock.assert_async().await;
}

// ── rewrite_file_url (via rewrite_simple_json) ────────────────────────────

#[test]
fn rewrite_simple_json_leaves_slashless_url_unchanged() {
    let json = serde_json::json!({
        "files": [
            { "filename": "foo", "url": "no-slash-url" }
        ]
    });
    let body = serde_json::to_vec(&json).unwrap();
    let out = rewrite_simple_page(
        &body,
        Some("application/vnd.pypi.simple.v1+json"),
        "my-pypi",
        "http://localhost",
    );
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["files"][0]["url"].as_str().unwrap(), "no-slash-url");
}
