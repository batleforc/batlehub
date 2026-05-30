//! Integration tests for self-hosted / private-CA HTTPS registry support.
//!
//! Each test focuses on one aspect of `UpstreamHttpOptions`:
//!
//! - **Bearer token** — `Authorization: Bearer <tok>` sent on every upstream request
//! - **Basic auth**   — `Authorization: Basic <base64>` sent per-request
//! - **Custom header** — arbitrary header (e.g. `X-Private-Token`) sent on every request
//! - **Custom CA cert** — client trusts a private CA and succeeds over HTTPS
//! - **Untrusted CA** — connection is rejected when the server cert is not in the trust store
//!
//! Auth tests use an HTTP mockito server to inspect request headers without TLS overhead.
//!
//! HTTPS tests spin up an in-process TLS server with a `rcgen`-generated self-signed cert.
//! The `GoProxyRegistryClient` is the chosen client for all tests because its `@latest`
//! endpoint makes a single GET that returns a compact JSON body — easy to mock.

use std::sync::Arc;

use batlehub_adapters::registry::{GoProxyRegistryClient, UpstreamHttpOptions};
use batlehub_core::{entities::PackageId, error::CoreError, ports::RegistryClient};
use mockito::Matcher;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Minimal JSON body that satisfies `GoProxyRegistryClient::resolve_metadata`.
const GOPROXY_LATEST_BODY: &str = r#"{"Version":"v0.1.0","Time":"2024-01-01T00:00:00Z"}"#;

/// Package used by all tests — an arbitrary Go module at `latest`.
fn test_pkg() -> PackageId {
    PackageId::new("go", "golang.org/x/text", "latest")
}

/// Start an in-process TLS server that serves `GOPROXY_LATEST_BODY` for every request.
/// Returns the bound address; the server runs until the Tokio runtime exits.
async fn spawn_tls_server(
    cert_der: CertificateDer<'static>,
    key_der: Vec<u8>,
) -> std::net::SocketAddr {
    // Use an explicit provider: sqlx pulls in aws-lc-rs and reqwest pulls in ring,
    // so both are in the dependency graph and rustls cannot auto-select one.
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let tls_config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("default TLS versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![cert_der],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der)),
        )
        .expect("invalid TLS configuration");

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(stream).await else {
                    return;
                };
                // Drain the HTTP request so reqwest can finish writing it.
                let mut buf = [0u8; 4096];
                let _ = tls.read(&mut buf).await;
                // Send a minimal HTTP/1.1 200 response with the goproxy body.
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: application/json\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\r\n{}",
                    GOPROXY_LATEST_BODY.len(),
                    GOPROXY_LATEST_BODY,
                );
                let _ = tls.write_all(response.as_bytes()).await;
            });
        }
    });

    addr
}

// ── Auth header tests (plain HTTP, mockito) ───────────────────────────────────

/// A bearer token must appear as `Authorization: Bearer <tok>` on every upstream request.
#[tokio::test]
async fn bearer_token_forwarded_to_upstream() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/golang.org/x/text/@latest")
        .match_header("authorization", "Bearer s3cr3t-t0k3n")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    let opts = UpstreamHttpOptions {
        bearer_token: Some("s3cr3t-t0k3n".to_owned()),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(server.url(), &opts).unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;
    assert!(result.is_ok(), "resolve_metadata failed: {:?}", result);
    mock.assert_async().await;
}

/// Basic-auth credentials must appear as `Authorization: Basic <base64(user:pass)>`.
/// The exact base64 payload is reqwest's responsibility; we assert the scheme.
#[tokio::test]
async fn basic_auth_forwarded_to_upstream() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/golang.org/x/text/@latest")
        .match_header(
            "authorization",
            Matcher::Regex("^Basic [A-Za-z0-9+/]+=*$".to_string()),
        )
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    let opts = UpstreamHttpOptions {
        basic_auth: Some(("alice".to_owned(), "hunter2".to_owned())),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(server.url(), &opts).unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;
    assert!(result.is_ok(), "resolve_metadata failed: {:?}", result);
    mock.assert_async().await;
}

/// A custom header (e.g. `X-Private-Token`) must appear on every upstream request.
#[tokio::test]
async fn custom_header_forwarded_to_upstream() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("GET", "/golang.org/x/text/@latest")
        .match_header("x-private-token", "tok-abc123")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    let opts = UpstreamHttpOptions {
        custom_header: Some(("x-private-token".to_owned(), "tok-abc123".to_owned())),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(server.url(), &opts).unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;
    assert!(result.is_ok(), "resolve_metadata failed: {:?}", result);
    mock.assert_async().await;
}

// ── HTTPS / custom-CA tests ───────────────────────────────────────────────────

/// A registry client built with a `ca_cert_path` can connect to an upstream that
/// presents a certificate signed by that CA.  The connection must succeed end-to-end
/// up to a valid `PackageMetadata` being returned.
#[tokio::test]
async fn custom_ca_cert_enables_https_connection() {
    // 1. Generate a self-signed certificate valid for "localhost".
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("rcgen cert generation");
    let cert_der = certified.cert.der().clone();
    let key_der = certified.signing_key.serialize_der();
    let cert_pem = certified.cert.pem();

    // 2. Start the HTTPS server with that certificate.
    let addr = spawn_tls_server(cert_der, key_der).await;

    // 3. Write the CA cert (same as server cert for self-signed) to a temp file.
    let ca_path = std::env::temp_dir().join(format!(
        "batlehub-test-ca-{}.pem",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    ));
    std::fs::write(&ca_path, cert_pem.as_bytes()).expect("write CA file");

    // 4. Build a registry client whose internal reqwest client trusts the CA.
    //    `GoProxyRegistryClient::new` calls `apply_upstream_options` →
    //    `apply_upstream_tls`, which loads the cert from `ca_cert_path`.
    let opts = UpstreamHttpOptions {
        ca_cert_path: Some(ca_path.to_str().unwrap().to_owned()),
        ..Default::default()
    };
    let client =
        GoProxyRegistryClient::new(format!("https://localhost:{}", addr.port()), &opts).unwrap();

    // 5. Make a real HTTPS request through the registry client.
    let result = client.resolve_metadata(&test_pkg()).await;
    let _ = std::fs::remove_file(&ca_path);

    assert!(
        result.is_ok(),
        "expected successful HTTPS connection with custom CA, got: {:?}",
        result
    );
    assert_eq!(result.unwrap().id.version, "v0.1.0");
}

/// Without the custom CA in the trust store the TLS handshake must fail.
/// The error must propagate as `CoreError::Registry` (a connection error),
/// not silently succeed or return a different error kind.
#[tokio::test]
async fn untrusted_ca_cert_rejects_https_connection() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("rcgen cert generation");
    let cert_der = certified.cert.der().clone();
    let key_der = certified.signing_key.serialize_der();

    let addr = spawn_tls_server(cert_der, key_der).await;

    // No `ca_cert_path` — the system CA store does not contain our self-signed cert.
    let client = GoProxyRegistryClient::new(
        format!("https://localhost:{}", addr.port()),
        &Default::default(),
    )
    .unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;

    assert!(
        result.is_err(),
        "expected TLS error for untrusted CA, but request succeeded"
    );
    assert!(
        matches!(result, Err(CoreError::Registry(_))),
        "expected CoreError::Registry wrapping a TLS error, got: {:?}",
        result
    );
}
