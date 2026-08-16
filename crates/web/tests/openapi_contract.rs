//! The contract gate (RFC 0004 §6.1, §10).
//!
//! Every documented success response must declare a body. A `200` with only a
//! `description` makes `@hey-api/openapi-ts` emit `unknown`, which is why
//! `ui/` used to hand-write mirrors of four DTOs, and why the docs site's API
//! reference is blank for those endpoints.
//!
//! This walks the generated `ApiDoc` rather than linting the source, so it
//! fails on the next undocumented response and cannot be satisfied by a
//! comment.

use utoipa::openapi::path::Operation;
use utoipa::openapi::{PathItem, RefOr, Response};

/// Statuses that must carry a schema. `204` is excluded because "no content"
/// is the body.
const MUST_HAVE_BODY: [&str; 2] = ["200", "201"];

fn response_has_schema(response: &Response) -> bool {
    response
        .content
        .values()
        .any(|content| content.schema.is_some())
}

fn operations(item: &PathItem) -> Vec<(&'static str, &Operation)> {
    [
        ("GET", item.get.as_ref()),
        ("PUT", item.put.as_ref()),
        ("POST", item.post.as_ref()),
        ("DELETE", item.delete.as_ref()),
        ("OPTIONS", item.options.as_ref()),
        ("HEAD", item.head.as_ref()),
        ("PATCH", item.patch.as_ref()),
        ("TRACE", item.trace.as_ref()),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.map(|operation| (method, operation)))
    .collect()
}

fn offenders_for(path: &str, item: &PathItem) -> Vec<String> {
    let mut out = Vec::new();
    for (method, operation) in operations(item) {
        for (status, response) in &operation.responses.responses {
            if !MUST_HAVE_BODY.contains(&status.as_str()) {
                continue;
            }
            let missing = match response {
                RefOr::T(response) => !response_has_schema(response),
                // A `$ref` to a shared component response is a declared body.
                RefOr::Ref(_) => false,
            };
            if missing {
                out.push(format!("{method} {path} -> {status}"));
            }
        }
    }
    out
}

#[test]
fn every_success_response_declares_a_body() {
    let spec = batlehub_web::openapi_spec();

    let mut offenders: Vec<String> = spec
        .paths
        .paths
        .iter()
        .flat_map(|(path, item)| offenders_for(path, item))
        .collect();
    offenders.sort();

    assert!(
        offenders.is_empty(),
        "{} documented success response(s) declare no body. \
         Add `body = T` to the `utoipa::path` responses(...) clause; where the \
         handler returns an ad-hoc `json!`, give it a named DTO deriving \
         `ToSchema` in the same module (RFC 0004 §6.1).\n{}",
        offenders.len(),
        offenders.join("\n"),
    );
}

/// A guard on the guard: if route collection ever silently returns an empty
/// spec, the assertion above would pass vacuously.
#[test]
fn the_spec_is_not_empty() {
    let spec = batlehub_web::openapi_spec();
    assert!(
        spec.paths.paths.len() > 100,
        "expected the full route set, got {} paths",
        spec.paths.paths.len(),
    );
}
