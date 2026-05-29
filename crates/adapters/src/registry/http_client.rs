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
        let pem =
            std::fs::read(path).map_err(|e| anyhow::anyhow!("reading CA cert '{}': {e}", path))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_opts() -> UpstreamHttpOptions {
        UpstreamHttpOptions::default()
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
}
