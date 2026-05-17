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
}

/// Applies only the TLS options from `opts` to `builder`, returning the
/// modified builder so callers can add their own default headers before `.build()`.
pub fn apply_upstream_tls(
    mut builder: reqwest::ClientBuilder,
    opts: &UpstreamHttpOptions,
) -> anyhow::Result<reqwest::ClientBuilder> {
    if let Some(ref path) = opts.ca_cert_path {
        let pem = std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("reading CA cert '{}': {e}", path))?;
        let cert = reqwest::Certificate::from_pem(&pem)
            .map_err(|e| anyhow::anyhow!("parsing CA cert '{}': {e}", path))?;
        builder = builder.add_root_certificate(cert);
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
