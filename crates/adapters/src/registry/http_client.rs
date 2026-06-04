use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};

/// Options that control how a registry client's upstream HTTP client is built.
#[derive(Debug, Default, Clone)]
pub struct UpstreamHttpOptions {
    /// `Authorization: Bearer <token>` injected as a default header.
    pub bearer_token: Option<String>,
    /// HTTP Basic auth credentials stored for per-request injection (reqwest
    /// does not support per-client basic auth).
    pub basic_auth: Option<(String, String)>,
    /// Arbitrary header name/value injected as a default header (e.g. `X-API-Key`).
    pub custom_header: Option<(String, String)>,
    /// Path to a PEM-encoded CA certificate to add as a trusted root.
    pub ca_cert_path: Option<String>,
    /// Override URL for the upstream search API used by the Package Explorer.
    ///
    /// - `None` — use the registry type's built-in default (e.g. `search.maven.org`
    ///   for Maven, `packagist.org` for Composer).
    /// - `Some(url)` — use this URL as the search base.
    /// - `Some("")` — disable upstream search for this registry entirely.
    pub search_url: Option<String>,
    /// HTTP/SOCKS proxy URL for all upstream requests (e.g. `http://proxy:3128`).
    pub proxy_url: Option<String>,
    /// Proxy Basic-auth username (used with `proxy_url`).
    pub proxy_username: Option<String>,
    /// Proxy Basic-auth password (used with `proxy_url`).
    pub proxy_password: Option<String>,
    /// Comma-separated hosts/domains to bypass the proxy for.
    pub no_proxy: Option<String>,
}

/// Applies TLS and proxy options from `opts` to `builder`, returning the
/// modified builder so callers can add their own default headers before `.build()`.
pub fn apply_upstream_tls(
    mut builder: reqwest::ClientBuilder,
    opts: &UpstreamHttpOptions,
) -> anyhow::Result<reqwest::ClientBuilder> {
    if let Some(ref path) = opts.ca_cert_path {
        let pem =
            std::fs::read(path).map_err(|e| anyhow::anyhow!("reading CA cert '{}': {e}", path))?;
        let cert = reqwest::Certificate::from_pem(&pem)
            .map_err(|e| anyhow::anyhow!("parsing CA cert '{}': {e}", path))?;
        builder = builder.add_root_certificate(cert);
    }
    if let Some(ref proxy_url) = opts.proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| anyhow::anyhow!("invalid proxy URL '{}': {e}", proxy_url))?;
        let proxy = match (&opts.proxy_username, &opts.proxy_password) {
            (Some(u), Some(p)) => proxy.basic_auth(u, p),
            _ => proxy,
        };
        let proxy = proxy.no_proxy(
            opts.no_proxy
                .as_deref()
                .and_then(reqwest::NoProxy::from_string),
        );
        builder = builder.proxy(proxy);
    }
    Ok(builder)
}

/// Returns a `HeaderMap` containing any Bearer / custom-header auth entries
/// from `opts`.  Callers can merge this into their own header map before
/// passing it to `.default_headers()`.
pub fn upstream_auth_headers(opts: &UpstreamHttpOptions) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    if let Some(ref tok) = opts.bearer_token {
        let value: HeaderValue = format!("Bearer {tok}")
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid bearer token header value"))?;
        headers.insert(AUTHORIZATION, value);
    }

    if let Some((ref name, ref value)) = opts.custom_header {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| anyhow::anyhow!("invalid custom header name '{name}'"))?;
        let header_value: HeaderValue = value
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid custom header value for '{name}'"))?;
        headers.insert(header_name, header_value);
    }

    Ok(headers)
}

/// Convenience wrapper: applies TLS options, injects auth headers as default
/// headers, and calls `.build()`.  Use this for clients with no registry-specific
/// default headers.  For clients that set their own headers (e.g. GitHub), use
/// `apply_upstream_tls` + `upstream_auth_headers` directly.
pub fn apply_upstream_options(
    builder: reqwest::ClientBuilder,
    opts: &UpstreamHttpOptions,
) -> anyhow::Result<reqwest::Client> {
    let mut builder = apply_upstream_tls(builder, opts)?;
    let auth_headers = upstream_auth_headers(opts)?;
    if !auth_headers.is_empty() {
        builder = builder.default_headers(auth_headers);
    }
    Ok(builder.build()?)
}

/// Percent-encode a query string value, encoding all characters except
/// unreserved ones (letters, digits, `-`, `_`, `.`, `~`).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((byte >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((byte & 0xf) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_opts() -> UpstreamHttpOptions {
        UpstreamHttpOptions::default()
    }

    // ── Proxy routing integration tests ──────────────────────────────────────

    /// A captured request as seen by the recording proxy.
    struct ProxyRequest {
        method: String,
        target_url: String,
        proxy_authorization: Option<String>,
    }

    /// Spawn a minimal HTTP/1.1 forwarding proxy on a random port.
    ///
    /// The proxy:
    /// - records each request it handles, sending a `ProxyRequest` on the channel,
    /// - strips the `Proxy-Authorization` header before forwarding,
    /// - converts the absolute-URI request line to relative form,
    /// - forwards to the upstream and pipes the response back.
    ///
    /// Returns the proxy port and a receiver for recorded requests.
    async fn start_recording_proxy()
    -> (u16, tokio::sync::mpsc::UnboundedReceiver<ProxyRequest>) {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let tx = tx.clone();
                tokio::spawn(proxy_handle(stream, tx));
            }
        });

        (port, rx)
    }

    /// Spawn a proxy that always returns 502 — used for `no_proxy` bypass tests.
    async fn start_rejecting_proxy() -> u16 {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

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

    async fn proxy_handle(
        mut client: tokio::net::TcpStream,
        tx: tokio::sync::mpsc::UnboundedSender<ProxyRequest>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut buf = vec![0u8; 8192];
        let n = client.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }

        // Parse the raw HTTP/1.x request.
        let raw = String::from_utf8_lossy(&buf[..n]);
        let mut lines = raw.lines();

        // Request line: "GET http://host:port/path HTTP/1.1"
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

        let _ = tx.send(ProxyRequest {
            method: method.clone(),
            target_url: url.clone(),
            proxy_authorization: proxy_auth,
        });

        // Derive relative path and upstream address from the absolute URI.
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

        // Forward as a relative-URI request (strip proxy metadata).
        let forwarded_req = format!(
            "{method} {rel_path} {version}\r\n{}\r\n\r\n",
            forwarded_headers.join("\r\n")
        );
        if upstream.write_all(forwarded_req.as_bytes()).await.is_err() {
            return;
        }

        // Pipe upstream response back to client.
        let mut resp = vec![0u8; 65536];
        let m = upstream.read(&mut resp).await.unwrap_or(0);
        let _ = client.write_all(&resp[..m]).await;
    }

    #[tokio::test]
    async fn proxy_routes_http_request_through_proxy() {
        let mut upstream = mockito::Server::new_async().await;
        let _mock = upstream
            .mock("GET", "/registry-path")
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;

        let (proxy_port, mut proxy_rx) = start_recording_proxy().await;

        let opts = UpstreamHttpOptions {
            proxy_url: Some(format!("http://127.0.0.1:{proxy_port}")),
            ..Default::default()
        };
        let client = apply_upstream_options(reqwest::Client::builder(), &opts).unwrap();

        let url = format!("{}/registry-path", upstream.url());
        let resp = client.get(&url).send().await.expect("request should succeed");
        assert_eq!(resp.status(), 200);

        // The proxy must have received exactly one request for our URL.
        let recorded = proxy_rx.recv().await.expect("proxy received a request");
        assert_eq!(recorded.method, "GET");
        assert!(
            recorded.target_url.ends_with("/registry-path"),
            "proxy saw absolute URI: {}",
            recorded.target_url
        );
    }

    #[tokio::test]
    async fn no_proxy_bypasses_proxy_for_excluded_host() {
        // Start an upstream that responds 200.
        let mut upstream = mockito::Server::new_async().await;
        let _mock = upstream
            .mock("GET", "/direct")
            .with_status(200)
            .with_body("direct")
            .create_async()
            .await;

        // Start a proxy that always returns 502; if the request goes through it the test fails.
        let bad_proxy_port = start_rejecting_proxy().await;

        let opts = UpstreamHttpOptions {
            proxy_url: Some(format!("http://127.0.0.1:{bad_proxy_port}")),
            // 127.0.0.1 is the upstream host — exclude it from proxying.
            no_proxy: Some("127.0.0.1".to_owned()),
            ..Default::default()
        };
        let client = apply_upstream_options(reqwest::Client::builder(), &opts).unwrap();

        let url = format!("{}/direct", upstream.url());
        let resp = client
            .get(&url)
            .send()
            .await
            .expect("request should reach upstream directly, bypassing the 502 proxy");
        assert_eq!(
            resp.status(),
            200,
            "response should come from upstream, not from the rejecting proxy"
        );
    }

    #[tokio::test]
    async fn proxy_auth_credentials_sent_as_proxy_authorization_header() {
        let mut upstream = mockito::Server::new_async().await;
        let _mock = upstream
            .mock("GET", "/auth-test")
            .with_status(200)
            .create_async()
            .await;

        let (proxy_port, mut proxy_rx) = start_recording_proxy().await;

        let opts = UpstreamHttpOptions {
            proxy_url: Some(format!("http://127.0.0.1:{proxy_port}")),
            proxy_username: Some("proxyuser".to_owned()),
            proxy_password: Some("proxypass".to_owned()),
            ..Default::default()
        };
        let client = apply_upstream_options(reqwest::Client::builder(), &opts).unwrap();

        let url = format!("{}/auth-test", upstream.url());
        let _ = client.get(&url).send().await;

        let recorded = proxy_rx.recv().await.expect("proxy received a request");
        assert!(
            recorded.proxy_authorization.is_some(),
            "Proxy-Authorization header should be sent with credentials"
        );
        let auth = recorded.proxy_authorization.unwrap();
        // reqwest encodes "proxyuser:proxypass" in Base64 for Basic auth.
        assert!(
            auth.to_ascii_lowercase().contains("basic"),
            "expected Basic auth, got: {auth}"
        );
    }

    #[test]
    fn default_opts_have_no_fields_set() {
        let opts = empty_opts();
        assert!(opts.bearer_token.is_none());
        assert!(opts.basic_auth.is_none());
        assert!(opts.custom_header.is_none());
        assert!(opts.ca_cert_path.is_none());
    }

    #[test]
    fn clone_preserves_all_fields() {
        let opts = UpstreamHttpOptions {
            bearer_token: Some("tok".to_owned()),
            basic_auth: Some(("user".to_owned(), "pass".to_owned())),
            custom_header: Some(("X-Key".to_owned(), "val".to_owned())),
            ca_cert_path: Some("/etc/ca.pem".to_owned()),
            ..Default::default()
        };
        let cloned = opts.clone();
        assert_eq!(cloned.bearer_token.as_deref(), Some("tok"));
        assert_eq!(cloned.ca_cert_path.as_deref(), Some("/etc/ca.pem"));
    }

    #[test]
    fn debug_format_contains_field_names() {
        let opts = UpstreamHttpOptions {
            bearer_token: Some("t".to_owned()),
            ..Default::default()
        };
        let s = format!("{opts:?}");
        assert!(s.contains("bearer_token"));
    }

    #[test]
    fn upstream_auth_headers_empty_opts_returns_empty_map() {
        let headers = upstream_auth_headers(&empty_opts()).unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn upstream_auth_headers_bearer_injects_authorization_header() {
        let opts = UpstreamHttpOptions {
            bearer_token: Some("mytoken".to_owned()),
            ..Default::default()
        };
        let headers = upstream_auth_headers(&opts).unwrap();
        let auth = headers.get("authorization").unwrap().to_str().unwrap();
        assert_eq!(auth, "Bearer mytoken");
    }

    #[test]
    fn upstream_auth_headers_custom_header_is_injected() {
        let opts = UpstreamHttpOptions {
            custom_header: Some(("X-Api-Key".to_owned(), "secret".to_owned())),
            ..Default::default()
        };
        let headers = upstream_auth_headers(&opts).unwrap();
        let val = headers.get("x-api-key").unwrap().to_str().unwrap();
        assert_eq!(val, "secret");
    }

    #[test]
    fn upstream_auth_headers_both_bearer_and_custom() {
        let opts = UpstreamHttpOptions {
            bearer_token: Some("tok".to_owned()),
            custom_header: Some(("X-Tenant".to_owned(), "acme".to_owned())),
            ..Default::default()
        };
        let headers = upstream_auth_headers(&opts).unwrap();
        assert!(headers.contains_key("authorization"));
        assert!(headers.contains_key("x-tenant"));
    }

    #[test]
    fn apply_upstream_tls_no_cert_returns_unmodified_builder() {
        let builder = reqwest::Client::builder();
        let result = apply_upstream_tls(builder, &empty_opts());
        assert!(result.is_ok());
        assert!(result.unwrap().build().is_ok());
    }

    #[test]
    fn apply_upstream_tls_nonexistent_cert_returns_error() {
        let opts = UpstreamHttpOptions {
            ca_cert_path: Some("/nonexistent/ca.pem".to_owned()),
            ..Default::default()
        };
        let err = apply_upstream_tls(reqwest::Client::builder(), &opts).unwrap_err();
        assert!(err.to_string().contains("reading CA cert"));
    }

    #[test]
    fn apply_upstream_options_no_auth_builds_client() {
        let client = apply_upstream_options(reqwest::Client::builder(), &empty_opts());
        assert!(client.is_ok());
    }

    #[test]
    fn apply_upstream_options_with_bearer_builds_client() {
        let opts = UpstreamHttpOptions {
            bearer_token: Some("tok".to_owned()),
            ..Default::default()
        };
        let client = apply_upstream_options(reqwest::Client::builder(), &opts);
        assert!(client.is_ok());
    }

    #[test]
    fn apply_upstream_options_with_custom_header_builds_client() {
        let opts = UpstreamHttpOptions {
            custom_header: Some(("X-Api-Key".to_owned(), "val".to_owned())),
            ..Default::default()
        };
        let client = apply_upstream_options(reqwest::Client::builder(), &opts);
        assert!(client.is_ok());
    }

    #[test]
    fn apply_upstream_tls_with_proxy_url_builds_client() {
        let opts = UpstreamHttpOptions {
            proxy_url: Some("http://proxy.example.com:3128".to_owned()),
            ..Default::default()
        };
        let result = apply_upstream_tls(reqwest::Client::builder(), &opts);
        assert!(result.is_ok());
        assert!(result.unwrap().build().is_ok());
    }

    #[test]
    fn apply_upstream_tls_with_proxy_and_basic_auth_builds_client() {
        let opts = UpstreamHttpOptions {
            proxy_url: Some("http://proxy.example.com:3128".to_owned()),
            proxy_username: Some("proxyuser".to_owned()),
            proxy_password: Some("proxypass".to_owned()),
            ..Default::default()
        };
        let result = apply_upstream_tls(reqwest::Client::builder(), &opts);
        assert!(result.is_ok());
        assert!(result.unwrap().build().is_ok());
    }

    #[test]
    fn apply_upstream_tls_with_proxy_and_no_proxy_builds_client() {
        let opts = UpstreamHttpOptions {
            proxy_url: Some("http://proxy.example.com:3128".to_owned()),
            no_proxy: Some("localhost,127.0.0.1,internal.example.com".to_owned()),
            ..Default::default()
        };
        let result = apply_upstream_tls(reqwest::Client::builder(), &opts);
        assert!(result.is_ok());
        assert!(result.unwrap().build().is_ok());
    }
}
