use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::ConfigFile;

use super::BatleHubClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResponse {
    pub user_id: Option<String>,
    pub role: String,
    pub auth_provider: Option<String>,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenListItem {
    pub id: Uuid,
    pub name: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTokenResponse {
    pub id: Uuid,
    pub name: String,
    pub token: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct CreateTokenRequest {
    pub name: String,
    pub expires_in_days: u64,
    pub role: String,
}

impl BatleHubClient {
    pub async fn whoami(&self) -> Result<MeResponse> {
        self.get("/api/v1/me").await
    }

    pub async fn list_tokens(&self) -> Result<Vec<TokenListItem>> {
        self.get("/api/v1/auth/tokens").await
    }

    pub async fn create_token(&self, req: CreateTokenRequest) -> Result<CreateTokenResponse> {
        self.post("/api/v1/auth/tokens", &req).await
    }

    pub async fn revoke_token(&self, id: Uuid) -> Result<()> {
        self.delete(&format!("/api/v1/auth/tokens/{id}")).await
    }
}

// ── OIDC helpers ───────────────────────────────────────────────────────────────

/// Fetch the IdP authorization URL by issuing a non-redirecting request to the
/// server's OIDC login endpoint and extracting the `Location` header.
pub async fn get_oidc_login_url(
    base_url: &str,
    csrf: &str,
    provider: Option<&str>,
) -> Result<String> {
    let client = reqwest::ClientBuilder::new()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let mut url = format!(
        "{base_url}/api/v1/auth/oidc/login?state={}",
        percent_encode(csrf)
    );
    if let Some(p) = provider {
        url.push_str(&format!("&provider={}", percent_encode(p)));
    }

    let resp = client.get(&url).send().await?;

    let status = resp.status();
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        anyhow::bail!("OIDC is not configured on this server");
    }
    if !status.is_redirection() {
        anyhow::bail!("OIDC login endpoint returned unexpected status {status}");
    }

    let location = resp
        .headers()
        .get("Location")
        .ok_or_else(|| {
            anyhow::anyhow!("Server did not return a redirect — OIDC may not be configured")
        })?
        .to_str()?
        .to_string();

    Ok(location)
}

/// List OIDC provider names that have browser SSO enabled.
pub async fn list_oidc_providers(client: &BatleHubClient) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Provider {
        name: String,
    }
    let providers: Vec<Provider> = client.get("/api/v1/auth/oidc/providers").await?;
    Ok(providers.into_iter().map(|p| p.name).collect())
}

/// Exchange a refresh token for a new access token via the server's OIDC refresh proxy.
/// Returns `(access_token, refresh_token, expires_in_seconds)`.
pub async fn oidc_refresh(
    base_url: &str,
    refresh_token: &str,
    provider: Option<&str>,
) -> Result<(String, Option<String>, Option<u64>)> {
    #[derive(Serialize)]
    struct Req<'a> {
        refresh_token: &'a str,
        provider: Option<&'a str>,
    }
    #[derive(Deserialize)]
    struct Resp {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/api/v1/auth/oidc/refresh"))
        .json(&Req {
            refresh_token,
            provider,
        })
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("OIDC refresh failed: {body}");
    }

    let r: Resp = resp.json().await?;
    Ok((r.access_token, r.refresh_token, r.expires_in))
}

/// Resolve the effective bearer token for the active profile:
/// - If a K8s token path is configured, reads it fresh from disk.
/// - If the OIDC access token is about to expire and a refresh token is stored,
///   performs the refresh, updates the config, and returns the new token.
/// - Otherwise returns the stored token as-is.
pub async fn resolve_token(
    base_url: &str,
    profile_name: Option<&str>,
    cfg: &mut ConfigFile,
) -> Result<Option<String>> {
    // Extract values without holding a borrow on cfg
    let (k8s_path, should_refresh, stored_token, stored_refresh, stored_provider) = {
        let p = profile_name
            .and_then(|n| cfg.profiles.get(n))
            .unwrap_or(&cfg.default);
        (
            p.kubernetes_token_path.clone(),
            p.is_token_expiring_soon() && p.oidc_refresh_token.is_some(),
            p.token.clone(),
            p.oidc_refresh_token.clone(),
            p.oidc_provider.clone(),
        )
    };

    if let Some(path) = k8s_path {
        return Ok(std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string()));
    }

    if should_refresh {
        if let Some(rt) = stored_refresh {
            match oidc_refresh(base_url, &rt, stored_provider.as_deref()).await {
                Ok((access_token, new_refresh, expires_in)) => {
                    let profile = match profile_name {
                        Some(n) => cfg.profiles.entry(n.to_string()).or_default(),
                        None => &mut cfg.default,
                    };
                    profile.token = Some(access_token.clone());
                    if let Some(nrt) = new_refresh {
                        profile.oidc_refresh_token = Some(nrt);
                    }
                    if let Some(exp) = expires_in {
                        profile.oidc_expires_at = Some(Utc::now().timestamp() + exp as i64);
                    }
                    cfg.save().ok();
                    return Ok(Some(access_token));
                }
                Err(e) => {
                    eprintln!("Warning: OIDC token refresh failed: {e}");
                }
            }
        }
    }

    Ok(stored_token)
}

/// Parse a token value or full SPA redirect URL pasted by the user after OIDC login.
/// Returns `(access_token, refresh_token, expires_at_unix)`.
pub fn parse_oidc_paste(input: &str) -> (String, Option<String>, Option<i64>) {
    if input.contains("oidc_access_token=") {
        let token = extract_param(input, "oidc_access_token").unwrap_or_else(|| input.to_string());
        let refresh = extract_param(input, "oidc_refresh_token");
        let expires_at = extract_param(input, "oidc_expires_in")
            .and_then(|s| s.parse::<i64>().ok())
            .map(|secs| Utc::now().timestamp() + secs);
        (token, refresh, expires_at)
    } else {
        (input.to_string(), None, None)
    }
}

fn extract_param(url: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = url.find(&needle)? + needle.len();
    let rest = &url[start..];
    let end = rest.find(['&', '#']).unwrap_or(rest.len());
    Some(url_decode(&rest[..end]))
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    i += 3;
                    continue;
                }
            }
        } else if bytes[i] == b'+' {
            decoded.push(b' ');
            i += 1;
            continue;
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn percent_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                vec![c]
            } else {
                c.to_string()
                    .bytes()
                    .flat_map(|b| format!("%{b:02X}").chars().collect::<Vec<_>>())
                    .collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_oidc_paste_bare_token() {
        let (token, refresh, expires) = parse_oidc_paste("mytoken123");
        assert_eq!(token, "mytoken123");
        assert!(refresh.is_none());
        assert!(expires.is_none());
    }

    #[test]
    fn parse_oidc_paste_full_url() {
        let url = "http://app.example.com/callback?\
                   oidc_access_token=ACCESS&oidc_refresh_token=REFRESH&oidc_expires_in=3600";
        let (token, refresh, expires) = parse_oidc_paste(url);
        assert_eq!(token, "ACCESS");
        assert_eq!(refresh.as_deref(), Some("REFRESH"));
        let now = Utc::now().timestamp();
        let exp = expires.unwrap();
        assert!(
            exp > now + 3500 && exp <= now + 3601,
            "expires_at={exp} should be near now+3600"
        );
    }

    #[test]
    fn parse_oidc_paste_url_encoded_token() {
        let url = "http://app.example.com/?oidc_access_token=tok%2Fbar%3Dbaz";
        let (token, _refresh, _expires) = parse_oidc_paste(url);
        assert_eq!(token, "tok/bar=baz");
    }

    #[test]
    fn parse_oidc_paste_no_refresh_no_expiry() {
        let url = "http://app.example.com/?oidc_access_token=ONLYACCESS";
        let (token, refresh, expires) = parse_oidc_paste(url);
        assert_eq!(token, "ONLYACCESS");
        assert!(refresh.is_none());
        assert!(expires.is_none());
    }
}
