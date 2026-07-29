//! Proxy trust, end to end (RFC 0001 §4.5, phase 0a).
//!
//! The unit tests beside `trusted_origin` cover the decision itself; these prove
//! the wiring — that a forwarded host only reaches a *generated* URL when the
//! peer is trusted. The NuGet service index is the vehicle because it is a plain
//! GET whose whole body is self-referencing `@id` URLs.
//!
//! See `tests/common/mod.rs` for the shared app-factory infrastructure.

mod common;
#[allow(unused_imports)]
use common::*;

use actix_web::test::{call_service, read_body_json, TestRequest};
use serde_json::Value;

use batlehub_config::schema::RegistryMode;

const INGRESS: &str = "10.42.0.7:5555";
const OUTSIDER: &str = "203.0.113.66:5555";

fn trusted() -> Option<Vec<String>> {
    Some(vec!["10.42.0.0/16".to_owned()])
}

/// Fetch the service index and return the `PackageBaseAddress` `@id`, which is
/// built from the request's own origin.
async fn service_index_base(
    trusted_proxies: Option<Vec<String>>,
    peer: &str,
    headers: &[(&str, &str)],
) -> String {
    let app = make_local_nuget_app_with_trust(RegistryMode::Local, trusted_proxies).await;
    let mut req = TestRequest::get()
        .uri("/proxy/local-nuget/nuget/v3/index.json")
        .peer_addr(peer.parse().unwrap())
        .insert_header(("Authorization", bearer(USER_TOKEN)));
    for (k, v) in headers {
        req = req.insert_header((*k, *v));
    }
    let resp = call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), 200);

    let body: Value = read_body_json(resp).await;
    body["resources"]
        .as_array()
        .expect("resources must be an array")
        .iter()
        .find(|r| {
            r["@type"]
                .as_str()
                .is_some_and(|t| t.starts_with("PackageBaseAddress"))
        })
        .and_then(|r| r["@id"].as_str())
        .expect("service index must expose a PackageBaseAddress resource")
        .to_owned()
}

#[actix_web::test]
async fn forwarded_host_from_a_trusted_ingress_is_used_in_generated_urls() {
    let base = service_index_base(
        trusted(),
        INGRESS,
        &[
            ("host", "batlehub.internal:8080"),
            ("x-forwarded-host", "hub.example.com"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert!(
        base.starts_with("https://hub.example.com/"),
        "a trusted ingress must be able to set the public origin, got: {base}"
    );
}

#[actix_web::test]
async fn spoofed_forwarded_host_from_an_untrusted_peer_is_ignored() {
    // The attack this phase closes: without a trust rule, this header would end
    // up in `@id` URLs the client then follows — and in cached response bodies.
    let base = service_index_base(
        trusted(),
        OUTSIDER,
        &[
            ("host", "hub.example.com"),
            ("x-forwarded-host", "evil.example.net"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert!(
        base.starts_with("http://hub.example.com/"),
        "an untrusted peer must not choose the origin, got: {base}"
    );
    assert!(
        !base.contains("evil.example.net"),
        "spoofed host leaked into a generated URL: {base}"
    );
}

#[actix_web::test]
async fn an_empty_trusted_list_ignores_forwarded_headers_from_everyone() {
    let base = service_index_base(
        Some(vec![]),
        INGRESS,
        &[
            ("host", "hub.example.com"),
            ("x-forwarded-host", "elsewhere.example.net"),
        ],
    )
    .await;
    assert!(
        base.starts_with("http://hub.example.com/"),
        "trusted_proxies = [] must ignore forwarded headers, got: {base}"
    );
}

#[actix_web::test]
async fn an_absent_list_reproduces_the_previous_behaviour() {
    // Existing deployments generate the same URLs they always did — the reason
    // absent is not the same as empty.
    let base = service_index_base(
        None,
        OUTSIDER,
        &[
            ("host", "batlehub.internal:8080"),
            ("x-forwarded-host", "hub.example.com"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert!(
        base.starts_with("https://hub.example.com/"),
        "an absent list must keep trusting forwarded host/scheme, got: {base}"
    );
}

#[actix_web::test]
async fn without_forwarded_headers_the_host_header_decides() {
    let base = service_index_base(trusted(), INGRESS, &[("host", "hub.example.com")]).await;
    assert!(base.starts_with("http://hub.example.com/"), "got: {base}");
}
