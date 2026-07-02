use super::super::http_client::to_registry_error;
use super::CoreError;

// ── PEP 503 name normalisation ────────────────────────────────────────────────

/// Normalise a PyPI package name per PEP 503: lower-case, collapse runs of
/// `[-_.]` into a single `-`.
pub fn normalize_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut result = String::with_capacity(lower.len());
    let mut prev_dash = false;
    for ch in lower.chars() {
        if ch == '-' || ch == '_' || ch == '.' {
            if !prev_dash {
                result.push('-');
                prev_dash = true;
            }
        } else {
            result.push(ch);
            prev_dash = false;
        }
    }
    result
}

/// Fetch the Simple API HTML (or JSON) page for a package from the upstream.
///
/// Returns the raw body bytes and the `Content-Type` header value so the
/// handler can forward it to the client after URL rewriting.
pub async fn fetch_simple_page(
    client: &reqwest::Client,
    base_url: &str,
    name: &str,
    basic_auth: Option<&(String, String)>,
    accept: Option<&str>,
) -> Result<(bytes::Bytes, Option<String>), CoreError> {
    let normalized = normalize_name(name);
    let url = format!("{}/simple/{}/", base_url.trim_end_matches('/'), normalized);

    let mut builder = client.get(&url);
    if let Some((u, p)) = basic_auth {
        builder = builder.basic_auth(u, Some(p));
    }
    if let Some(accept_val) = accept {
        builder = builder.header(reqwest::header::ACCEPT, accept_val);
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| CoreError::Registry(format!("pypi: simple page request failed: {e}")))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CoreError::NotFound(format!(
            "pypi: package '{}' not found in simple index",
            name
        )));
    }
    if !resp.status().is_success() {
        return Err(CoreError::Registry(format!(
            "pypi: simple index returned {}",
            resp.status()
        )));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let body = resp.bytes().await.map_err(to_registry_error)?;

    Ok((body, content_type))
}

/// Rewrite href/url values in a PyPI simple page so all file links go through
/// the batlehub proxy at `/proxy/{registry}/packages/{filename}`.
///
/// Handles both HTML (PEP 503) and JSON (PEP 691) formats.
pub fn rewrite_simple_page(
    body: &[u8],
    content_type: Option<&str>,
    registry: &str,
    proxy_base: &str,
) -> Vec<u8> {
    let is_json = content_type
        .map(|ct| ct.contains("application/vnd.pypi.simple"))
        .unwrap_or(false);

    if is_json {
        rewrite_simple_json(body, registry, proxy_base)
    } else {
        rewrite_simple_html(body, registry, proxy_base)
    }
}

/// Rewrite one `href` value if it is an absolute HTTP URL pointing to a PyPI
/// CDN file. Returns `Some(rewritten)` when rewriting is applicable, `None`
/// when the original value should be kept unchanged.
fn rewrite_abs_href(href_value: &str, proxy_packages: &str) -> Option<String> {
    if !href_value.starts_with("https://") && !href_value.starts_with("http://") {
        return None;
    }
    if let Some(fragment_pos) = href_value.rfind('#') {
        let path_part = &href_value[..fragment_pos];
        let fragment = &href_value[fragment_pos..];
        if let Some(slash_pos) = path_part.rfind('/') {
            let filename = &path_part[slash_pos + 1..];
            return Some(format!("{proxy_packages}/{filename}{fragment}"));
        }
    } else if let Some(slash_pos) = href_value.rfind('/') {
        let filename = &href_value[slash_pos + 1..];
        return Some(format!("{proxy_packages}/{filename}"));
    }
    None
}

pub(super) fn rewrite_simple_html(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
    let text = match std::str::from_utf8(body) {
        Ok(s) => s,
        Err(_) => return body.to_vec(),
    };

    let proxy_packages = format!("{proxy_base}/proxy/{registry}/packages");
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(href_pos) = remaining.find("href=\"") {
        let after_quote = &remaining[href_pos + 6..];
        result.push_str(&remaining[..href_pos + 6]);

        if let Some(end_quote) = after_quote.find('"') {
            let href_value = &after_quote[..end_quote];
            remaining = &after_quote[end_quote..];
            let rewritten = rewrite_abs_href(href_value, &proxy_packages)
                .unwrap_or_else(|| href_value.to_owned());
            result.push_str(&rewritten);
        } else {
            remaining = after_quote;
        }
    }
    result.push_str(remaining);
    result.into_bytes()
}

pub(super) fn rewrite_simple_json(body: &[u8], registry: &str, proxy_base: &str) -> Vec<u8> {
    let mut json: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(_) => return body.to_vec(),
    };

    let proxy_packages = format!("{proxy_base}/proxy/{registry}/packages");

    if let Some(files) = json.get_mut("files").and_then(|f| f.as_array_mut()) {
        for file in files.iter_mut() {
            if let Some(url_val) = file.get_mut("url") {
                if let Some(url_str) = url_val.as_str() {
                    let rewritten = rewrite_file_url(url_str, &proxy_packages);
                    *url_val = serde_json::Value::String(rewritten);
                }
            }
        }
    }

    serde_json::to_vec(&json).unwrap_or_else(|_| body.to_vec())
}

fn rewrite_file_url(url: &str, proxy_packages: &str) -> String {
    // Split off fragment first
    let (path_part, fragment) = if let Some(frag_pos) = url.rfind('#') {
        (&url[..frag_pos], &url[frag_pos..])
    } else {
        (url, "")
    };

    if let Some(slash_pos) = path_part.rfind('/') {
        let filename = &path_part[slash_pos + 1..];
        format!("{proxy_packages}/{filename}{fragment}")
    } else {
        url.to_owned()
    }
}
