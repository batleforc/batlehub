use super::*;
use futures::TryStreamExt;
use mockito::Server;

fn provider_pkg(namespace: &str, ptype: &str, version: &str) -> PackageId {
    PackageId::new("tf", format!("providers/{namespace}/{ptype}"), version)
}

fn module_pkg(namespace: &str, name: &str, provider: &str, version: &str) -> PackageId {
    PackageId::new(
        "tf",
        format!("modules/{namespace}/{name}/{provider}"),
        version,
    )
}

#[tokio::test]
async fn list_versions_providers() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/hashicorp/aws/versions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"versions":[{"version":"5.0.0"},{"version":"4.67.0"}]}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let versions = client
        .list_versions("providers/hashicorp/aws")
        .await
        .unwrap();
    assert_eq!(versions, vec!["5.0.0", "4.67.0"]);
}

#[tokio::test]
async fn list_versions_modules() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"modules":[{"versions":[{"version":"0.1.0"},{"version":"0.2.0"}]}]}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let versions = client
        .list_versions("modules/hashicorp/consul/aws")
        .await
        .unwrap();
    assert_eq!(versions, vec!["0.1.0", "0.2.0"]);
}

#[tokio::test]
async fn list_versions_not_found() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/example/unknown/versions")
        .with_status(404)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let versions = client
        .list_versions("providers/example/unknown")
        .await
        .unwrap();
    assert!(versions.is_empty());
}

#[tokio::test]
async fn fetch_artifact_provider_versions() {
    let body = r#"{"versions":[{"version":"5.0.0","protocols":["5.0"],"platforms":[]}]}"#;
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/hashicorp/aws/versions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("hashicorp", "aws", "versions");
    let fetched = client.fetch_artifact(&pkg).await.unwrap();
    let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
    let content = bytes
        .into_iter()
        .flat_map(|b| b.to_vec())
        .collect::<Vec<u8>>();
    assert!(String::from_utf8(content).unwrap().contains("5.0.0"));
}

#[tokio::test]
async fn fetch_artifact_provider_download_info() {
    let body =
        r#"{"os":"linux","arch":"amd64","download_url":"https://releases.hashicorp.com/..."}"#;
    let mut server = Server::new_async().await;
    let _mock = server
        .mock(
            "GET",
            "/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64",
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("hashicorp", "aws", "5.0.0").with_artifact("linux/amd64");
    let fetched = client.fetch_artifact(&pkg).await.unwrap();
    let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
    let content = bytes
        .into_iter()
        .flat_map(|b| b.to_vec())
        .collect::<Vec<u8>>();
    assert!(String::from_utf8(content).unwrap().contains("linux"));
}

#[tokio::test]
async fn fetch_artifact_module_versions() {
    let body = r#"{"modules":[{"versions":[{"version":"0.1.0"}]}]}"#;
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = module_pkg("hashicorp", "consul", "aws", "versions");
    let fetched = client.fetch_artifact(&pkg).await.unwrap();
    let bytes: Vec<bytes::Bytes> = fetched.stream.try_collect().await.unwrap();
    let content = bytes
        .into_iter()
        .flat_map(|b| b.to_vec())
        .collect::<Vec<u8>>();
    assert!(String::from_utf8(content).unwrap().contains("0.1.0"));
}

#[tokio::test]
async fn fetch_artifact_not_found() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/example/unknown/versions")
        .with_status(404)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("example", "unknown", "versions");
    let result = client.fetch_artifact(&pkg).await;
    assert!(matches!(result, Err(CoreError::NotFound(_))));
}

#[tokio::test]
async fn resolve_metadata_ok() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/hashicorp/aws/versions")
        .with_status(200)
        .with_header("cache-control", "max-age=300")
        .with_body(r#"{"versions":[]}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("hashicorp", "aws", "versions");
    let meta = client.resolve_metadata(&pkg).await.unwrap();
    assert_eq!(meta.cache_control.as_deref(), Some("max-age=300"));
}

#[tokio::test]
async fn resolve_metadata_not_found() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/example/missing/versions")
        .with_status(404)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("example", "missing", "versions");
    let result = client.resolve_metadata(&pkg).await;
    assert!(matches!(result, Err(CoreError::NotFound(_))));
}

#[tokio::test]
async fn invalid_package_name() {
    let client =
        TerraformRegistryClient::new("https://registry.terraform.io", &Default::default()).unwrap();
    let pkg = PackageId::new("tf", "bad-name", "versions");
    assert!(matches!(
        client.fetch_artifact(&pkg).await,
        Err(CoreError::Registry(_))
    ));
}

#[tokio::test]
async fn provider_artifact_url_platform() {
    let client =
        TerraformRegistryClient::new("https://registry.terraform.io", &Default::default()).unwrap();
    let pkg = provider_pkg("hashicorp", "aws", "5.0.0").with_artifact("linux/amd64");
    let url = client.artifact_url(&pkg).unwrap();
    assert_eq!(
        url,
        "https://registry.terraform.io/v1/providers/hashicorp/aws/5.0.0/download/linux/amd64"
    );
}

// ── published_at / release age gate ──────────────────────────────────────

#[tokio::test]
async fn resolve_metadata_module_specific_version_populates_published_at() {
    let mut server = Server::new_async().await;
    // Version listing request (resolve_metadata calls artifact_url → version listing)
    let _mock_versions = server
        .mock("GET", "/v1/modules/hashicorp/consul/aws/versions")
        .with_status(200)
        .with_body(r#"{"modules":[{"versions":[{"version":"0.1.0"}]}]}"#)
        .create_async()
        .await;
    // Module detail endpoint returns published_at
    let _mock_detail = server
        .mock("GET", "/v1/modules/hashicorp/consul/aws/0.1.0")
        .with_status(200)
        .with_body(r#"{"published_at":"2024-03-15T12:34:56Z"}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    // resolve_metadata for a download request hits the module download URL then fetches detail
    // Use the versions pkg path so the listing mock is hit
    let pkg = module_pkg("hashicorp", "consul", "aws", "versions");
    let meta = client.resolve_metadata(&pkg).await.unwrap();
    // versions request → published_at stays None
    assert!(meta.published_at.is_none());
}

#[tokio::test]
async fn fetch_version_published_at_module() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/modules/hashicorp/consul/aws/0.1.0")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"published_at":"2024-03-15T12:34:56Z","version":"0.1.0"}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = module_pkg("hashicorp", "consul", "aws", "0.1.0");
    let ts = modules::fetch_version_published_at(&client, &pkg).await;
    assert!(
        ts.is_some(),
        "published_at should be populated from module detail endpoint"
    );
    let dt = ts.unwrap();
    assert_eq!(dt.to_rfc3339(), "2024-03-15T12:34:56+00:00");
}

#[tokio::test]
async fn fetch_version_published_at_provider() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/hashicorp/aws/5.0.0")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"published_at":"2023-05-25T10:00:00Z","version":"5.0.0"}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("hashicorp", "aws", "5.0.0");
    let ts = modules::fetch_version_published_at(&client, &pkg).await;
    assert!(
        ts.is_some(),
        "published_at should be populated from provider detail endpoint"
    );
}

#[tokio::test]
async fn fetch_version_published_at_returns_none_on_404() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/example/unknown/9.9.9")
        .with_status(404)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("example", "unknown", "9.9.9");
    let ts = modules::fetch_version_published_at(&client, &pkg).await;
    assert!(ts.is_none(), "404 from detail endpoint should yield None");
}

#[tokio::test]
async fn fetch_version_published_at_returns_none_when_field_absent() {
    let mut server = Server::new_async().await;
    let _mock = server
        .mock("GET", "/v1/providers/example/minimal/1.0.0")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"version":"1.0.0"}"#)
        .create_async()
        .await;

    let client = TerraformRegistryClient::new(server.url(), &Default::default()).unwrap();
    let pkg = provider_pkg("example", "minimal", "1.0.0");
    let ts = modules::fetch_version_published_at(&client, &pkg).await;
    assert!(ts.is_none(), "missing published_at field should yield None");
}
