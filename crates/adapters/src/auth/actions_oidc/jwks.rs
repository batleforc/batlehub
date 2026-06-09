use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::DecodingKey;
use serde::Deserialize;

pub(super) const JWKS_MIN_REFRESH: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
pub(super) struct OidcDiscovery {
    pub(super) issuer: String,
    pub(super) jwks_uri: String,
}

pub(super) struct JwksCache {
    pub(super) keys: JwkSet,
    pub(super) fetched_at: Instant,
}

pub(super) fn find_key(jwks: &JwkSet, kid: Option<&str>) -> Option<DecodingKey> {
    let jwk = if let Some(kid) = kid {
        jwks.find(kid)
    } else {
        jwks.keys.first()
    }?;
    DecodingKey::from_jwk(jwk).ok()
}

pub(super) async fn fetch_jwks(
    http: &reqwest::Client,
    uri: &str,
) -> Result<JwkSet, reqwest::Error> {
    http.get(uri).send().await?.json().await
}
