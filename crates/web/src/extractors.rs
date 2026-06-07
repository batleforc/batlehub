use std::collections::HashMap;

use actix_web::{dev::Payload, FromRequest, HttpMessage, HttpRequest};
use futures::future::{ready, Ready};

use batlehub_core::entities::Identity;

use crate::error::AppError;

/// Extracts the `Identity` attached by `AuthMiddleware` from request extensions.
///
/// Falls back to `Identity::anonymous()` if no middleware has run (should not
/// happen in production, but avoids panics in tests).
pub struct AuthIdentity(pub Identity);

impl FromRequest for AuthIdentity {
    type Error = AppError;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let identity = req
            .extensions()
            .get::<Identity>()
            .cloned()
            .unwrap_or_else(Identity::anonymous);
        ready(Ok(AuthIdentity(identity)))
    }
}

impl std::ops::Deref for AuthIdentity {
    type Target = Identity;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Builds a `RawAuthRequest` from the actix-web `HttpRequest`.
pub fn raw_auth_from_request(req: &HttpRequest) -> batlehub_core::ports::RawAuthRequest {
    let mut headers = req
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.to_string(), v.to_owned()))
        })
        .collect::<HashMap<_, _>>();

    // NuGet clients send X-NuGet-ApiKey instead of Authorization: Bearer.
    // Normalize so all auth providers see a standard Bearer token.
    if !headers.contains_key("authorization") {
        if let Some(key) = headers.get("x-nuget-apikey").cloned() {
            headers.insert("authorization".to_owned(), format!("Bearer {key}"));
        }
    }

    let query_params = req
        .query_string()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().filter(|k| !k.is_empty())?.to_owned();
            let val = parts.next().unwrap_or("").to_owned();
            Some((key, val))
        })
        .collect::<HashMap<_, _>>();

    batlehub_core::ports::RawAuthRequest {
        headers,
        query_params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test::TestRequest;

    #[test]
    fn extracts_authorization_header() {
        let req = TestRequest::get()
            .insert_header(("authorization", "Bearer mytoken123"))
            .to_http_request();
        let raw = raw_auth_from_request(&req);
        assert_eq!(
            raw.headers.get("authorization").map(String::as_str),
            Some("Bearer mytoken123")
        );
    }

    #[test]
    fn extracts_query_params() {
        let req = TestRequest::get()
            .uri("/?token=abc&foo=bar")
            .to_http_request();
        let raw = raw_auth_from_request(&req);
        assert_eq!(
            raw.query_params.get("token").map(String::as_str),
            Some("abc")
        );
        assert_eq!(raw.query_params.get("foo").map(String::as_str), Some("bar"));
    }

    #[test]
    fn no_query_string_yields_empty_params() {
        let req = TestRequest::get().uri("/").to_http_request();
        let raw = raw_auth_from_request(&req);
        assert!(
            raw.query_params.is_empty(),
            "empty query string must produce no params"
        );
    }

    #[test]
    fn trailing_ampersand_does_not_insert_empty_key() {
        let req = TestRequest::get().uri("/?token=abc&").to_http_request();
        let raw = raw_auth_from_request(&req);
        assert_eq!(
            raw.query_params.get("token").map(String::as_str),
            Some("abc")
        );
        assert!(
            !raw.query_params.contains_key(""),
            "trailing & must not insert empty key"
        );
    }
}
