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

// ── In-process HTTP proxy helpers ─────────────────────────────────────────────

struct ProxiedRequest {
    method: String,
    target_url: String,
    proxy_authorization: Option<String>,
}

/// Spawn a minimal transparent HTTP/1.1 forwarding proxy on a random port.
///
/// Behaves like tinyproxy / Squid with caching disabled: every request is
/// forwarded to the real upstream and the response is piped back unchanged.
/// Each handled request is reported on the returned channel so tests can assert
/// that traffic actually went through the proxy.
async fn spawn_recording_proxy() -> (u16, tokio::sync::mpsc::UnboundedReceiver<ProxiedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            tokio::spawn(proxy_forward(stream, tx));
        }
    });

    (port, rx)
}

/// Spawn a proxy that always returns 502 — used to prove `no_proxy` bypasses it.
async fn spawn_rejecting_proxy() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let _ = stream
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    });

    port
}

async fn proxy_forward(
    mut client: tokio::net::TcpStream,
    tx: tokio::sync::mpsc::UnboundedSender<ProxiedRequest>,
) {
    let mut buf = vec![0u8; 8192];
    let n = client.read(&mut buf).await.unwrap_or(0);
    if n == 0 {
        return;
    }

    let raw = String::from_utf8_lossy(&buf[..n]);
    let mut lines = raw.lines();

    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return;
    }
    let method = parts[0].to_owned();
    let url = parts[1].to_owned();
    let version = parts[2];

    let mut proxy_auth: Option<String> = None;
    let mut forwarded_headers: Vec<String> = Vec::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if line.to_ascii_lowercase().starts_with("proxy-authorization:") {
            proxy_auth = Some(line.to_owned());
        } else {
            forwarded_headers.push(line.to_owned());
        }
    }

    let _ = tx.send(ProxiedRequest {
        method: method.clone(),
        target_url: url.clone(),
        proxy_authorization: proxy_auth,
    });

    let without_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url.as_str());

    let (host_port, rel_path) = match without_scheme.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (without_scheme, "/".to_owned()),
    };
    let upstream_addr = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:80")
    };

    let mut upstream = match tokio::net::TcpStream::connect(&upstream_addr).await {
        Ok(s) => s,
        Err(_) => {
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
            return;
        }
    };

    let forwarded_req = format!(
        "{method} {rel_path} {version}\r\n{}\r\n\r\n",
        forwarded_headers.join("\r\n")
    );
    if upstream.write_all(forwarded_req.as_bytes()).await.is_err() {
        return;
    }

    let mut resp = vec![0u8; 65536];
    let m = upstream.read(&mut resp).await.unwrap_or(0);
    let _ = client.write_all(&resp[..m]).await;
}

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

// ── Proxy routing tests (GoProxyRegistryClient end-to-end) ───────────────────

/// A `GoProxyRegistryClient` configured with `proxy_url` must route its upstream
/// HTTP request through the proxy, not directly to the target.
///
/// This mirrors the behaviour of tinyproxy / Squid without caching: every request
/// is forwarded unconditionally and the real upstream response is returned.
#[tokio::test]
async fn goproxy_client_routes_through_http_proxy() {
    let mut upstream = mockito::Server::new_async().await;
    let _mock = upstream
        .mock("GET", "/golang.org/x/text/@latest")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    let (proxy_port, mut proxy_rx) = spawn_recording_proxy().await;

    let opts = UpstreamHttpOptions {
        proxy_url: Some(format!("http://127.0.0.1:{proxy_port}")),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(upstream.url(), &opts).unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;
    assert!(
        result.is_ok(),
        "resolve_metadata should succeed through proxy: {:?}",
        result
    );
    assert_eq!(result.unwrap().id.version, "v0.1.0");

    // The proxy must have seen exactly one request for our URL.
    let recorded = proxy_rx.recv().await.expect("proxy should have received a request");
    assert_eq!(recorded.method, "GET");
    assert!(
        recorded.target_url.contains("/golang.org/x/text/@latest"),
        "proxy should have seen the full upstream URL, got: {}",
        recorded.target_url
    );
    assert!(
        recorded.proxy_authorization.is_none(),
        "unauthenticated proxy should receive no Proxy-Authorization header"
    );
}

/// When `proxy_username` + `proxy_password` are set the client must send a
/// `Proxy-Authorization: Basic …` header with the credentials — this is how
/// tinyproxy / Squid enforce access control.
#[tokio::test]
async fn goproxy_client_sends_proxy_basic_auth() {
    let mut upstream = mockito::Server::new_async().await;
    let _mock = upstream
        .mock("GET", "/golang.org/x/text/@latest")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    let (proxy_port, mut proxy_rx) = spawn_recording_proxy().await;

    let opts = UpstreamHttpOptions {
        proxy_url: Some(format!("http://127.0.0.1:{proxy_port}")),
        proxy_username: Some("proxyuser".to_owned()),
        proxy_password: Some("proxypass".to_owned()),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(upstream.url(), &opts).unwrap();

    let _ = client.resolve_metadata(&test_pkg()).await;

    let recorded = proxy_rx.recv().await.expect("proxy should have received a request");
    let auth = recorded
        .proxy_authorization
        .expect("Proxy-Authorization header must be sent with credentials");
    assert!(
        auth.to_ascii_lowercase().contains("basic"),
        "expected Basic auth scheme in Proxy-Authorization, got: {auth}"
    );
}

/// When a host is listed in `no_proxy` the client must bypass the proxy entirely
/// and connect directly — the proxy (here: a 502 rejector) must not be contacted.
#[tokio::test]
async fn goproxy_client_bypasses_proxy_for_no_proxy_host() {
    let mut upstream = mockito::Server::new_async().await;
    let _mock = upstream
        .mock("GET", "/golang.org/x/text/@latest")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(GOPROXY_LATEST_BODY)
        .create_async()
        .await;

    // This proxy always returns 502; if a request reaches it the test fails.
    let bad_proxy_port = spawn_rejecting_proxy().await;

    let opts = UpstreamHttpOptions {
        proxy_url: Some(format!("http://127.0.0.1:{bad_proxy_port}")),
        no_proxy: Some("127.0.0.1".to_owned()),
        ..Default::default()
    };
    let client = GoProxyRegistryClient::new(upstream.url(), &opts).unwrap();

    let result = client.resolve_metadata(&test_pkg()).await;
    assert!(
        result.is_ok(),
        "resolve_metadata should reach upstream directly, bypassing the 502 proxy: {:?}",
        result
    );
    assert_eq!(result.unwrap().id.version, "v0.1.0");
}

// ── HTTPS / custom-CA tests ───────────────────────────────────────────────────

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
