//! The authorization diagnostics — RFC 0015 §4.8.
//!
//! §4.8 puts the CLI beside the console page deliberately: *"The same data is
//! available to `batlehub authz explain` … so this is not a reason to open a
//! browser."* An operator diagnosing a `403` is usually in a terminal already,
//! frequently on a machine with no browser at all, and the answer to "why was
//! this refused?" should not require one.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::BatleHubClient;

/// One verb the subject holds, and where it came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedVerb {
    pub action: String,
    /// The tier that granted it — `registry:npm1`, `namespace:@acme/billing`.
    ///
    /// **The point of the whole endpoint.** A resolved set without provenance
    /// tells an operator *what* they have; naming the tier and the subject form
    /// tells them which line to edit.
    pub granted_by: String,
    /// The subject form that matched, in grant spelling.
    pub subject: String,
}

/// The resource attributes that apply, composed across the tiers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Attributes {
    pub visibility: String,
    pub prerelease_visibility: String,
    pub immutable: String,
    pub monotonic: bool,
    pub versioning_dry_run: bool,
    pub exempt_gates: Vec<String>,
}

/// A node whose shadow is serving what its grants refuse (§4.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowNote {
    pub node: String,
    pub until: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainResponse {
    pub decision: String,
    pub reason: Option<String>,
    pub resolved: Vec<ResolvedVerb>,
    pub tiers_walked: Vec<String>,
    pub not_covered: Vec<String>,
    #[serde(default)]
    pub attributes: Attributes,
    #[serde(default)]
    pub shadowed_by: Option<ShadowNote>,
}

/// One node's shadow, summarised (§4.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowSummary {
    pub node: String,
    pub shadow_until: String,
    pub count: u64,
    pub actions: Vec<String>,
    pub subjects: Vec<String>,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowedDenial {
    pub at: String,
    pub registry: String,
    pub package: String,
    pub version: String,
    pub action: String,
    pub subject: String,
    pub node: String,
    pub shadow_until: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowResponse {
    pub by_node: Vec<ShadowSummary>,
    pub recent: Vec<ShadowedDenial>,
    pub kept: usize,
    pub no_shadow_configured: bool,
}

/// The query `explain` takes.
///
/// The resource is three fields rather than one string, and §4.8 records why:
/// *"a package name contains the separator a single string would have to be
/// split on"* — `@acme/billing/cards` for npm, `example.com/team/lib` for Go.
pub struct ExplainQuery<'a> {
    pub registry: &'a str,
    pub subject: &'a str,
    pub action: &'a str,
    pub package: Option<&'a str>,
    pub version: Option<&'a str>,
}

impl BatleHubClient {
    pub async fn authz_explain(&self, q: ExplainQuery<'_>) -> Result<ExplainResponse> {
        // The CLI's own encoder, shared with the OIDC flow rather than a second
        // one: `auth.rs` already learned which characters have to be escaped
        // here, and a subject like `group:oidc1:eng` or a package like
        // `@acme/billing/cards` puts every one of them in a query parameter.
        let enc = super::auth::percent_encode;
        let mut path = format!(
            "/api/v1/admin/authz/explain?registry={}&subject={}&action={}",
            enc(q.registry),
            enc(q.subject),
            enc(q.action)
        );
        if let Some(p) = q.package {
            path.push_str(&format!("&package={}", enc(p)));
        }
        if let Some(v) = q.version {
            path.push_str(&format!("&version={}", enc(v)));
        }
        self.get(&path).await
    }

    pub async fn authz_shadow(&self, limit: Option<usize>) -> Result<ShadowResponse> {
        let path = match limit {
            Some(n) => format!("/api/v1/admin/authz/shadow?limit={n}"),
            None => "/api/v1/admin/authz/shadow".to_owned(),
        };
        self.get(&path).await
    }
}
