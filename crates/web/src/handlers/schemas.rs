//! Shared response schemas for bodies that have no DTO of their own.
//!
//! RFC 0004 §4.1 requires every documented `200`/`201` to declare a body, and
//! `crates/web/tests/openapi_contract.rs` enforces it. Most handlers have a real
//! DTO to point at. Three kinds do not, and inventing a fake struct for them
//! would document a shape the server does not actually promise:
//!
//! - artifact bytes streamed from storage or an upstream,
//! - a protocol document forwarded from the configured upstream verbatim,
//! - a registry protocol document that is not JSON (XML, HTML, plain text).
//!
//! These three types say exactly that, once, so a generated client sees "opaque
//! bytes" or "opaque upstream document" instead of `unknown` — which is the
//! same lack of type with none of the explanation.
//!
//! The first three exist only to name a schema: handlers stream or forward the
//! payload directly, so nothing constructs them, and `#[schema(value_type = ...)]`
//! replaces the shape the wrapped type would otherwise emit.

use utoipa::ToSchema;

/// Raw artifact bytes — a tarball, wheel, `.crate`, `.nupkg`, `.gem`, `.vsix`,
/// package archive or signature — streamed from cache storage or from the
/// configured upstream.
///
/// The `Content-Type` is the one the registry protocol calls for; the payload is
/// never parsed by this server on the read path.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary, example = "<binary artifact>")]
pub struct ArtifactBytes(pub Vec<u8>);

/// A JSON document produced by the registry protocol rather than by this
/// server's own API.
///
/// In `proxy` mode it is the upstream's response forwarded (with URLs rewritten
/// where the protocol requires it); in `local` mode this server generates the
/// same protocol document. Either way the shape is the registry protocol's to
/// define, not this API's — npm's packument, NuGet's registration index and
/// Composer's `packages.json` are each specified by their own ecosystem, and
/// this server does not narrow them.
///
/// Documented as an unconstrained JSON value rather than an object: several of
/// these protocols answer with a top-level array, and a client that trusted a
/// fabricated object schema would be wrong about half of them. Where the shape
/// *is* known to be a list, the endpoint declares `Vec<UpstreamDocument>`.
#[derive(ToSchema)]
#[schema(value_type = Value)]
pub struct UpstreamDocument(pub serde_json::Value);

/// A non-JSON registry protocol document — `maven-metadata.xml`,
/// `updatePlugins.xml`, a PyPI simple-index HTML page, an APT `Release` file, a
/// Go `go.mod`, a plain-text token or version list.
///
/// As with [`UpstreamDocument`], the format belongs to the registry protocol.
/// The `Content-Type` header identifies which one.
#[derive(ToSchema)]
#[schema(value_type = String, example = "<registry protocol document>")]
pub struct ProtocolDocument(pub String);

/// `{"ok": true}` — the acknowledgement several write endpoints return when the
/// protocol they implement expects a JSON body but has nothing to say in it.
#[derive(serde::Serialize, ToSchema)]
pub struct OkResponse {
    /// Always `true`; a failure is reported by the status code, not by this field.
    pub ok: bool,
}

impl OkResponse {
    /// The body itself. Handlers serialise this rather than an ad-hoc `json!`,
    /// so the documented schema and the sent bytes cannot drift apart.
    pub fn new() -> Self {
        Self { ok: true }
    }
}

impl Default for OkResponse {
    fn default() -> Self {
        Self::new()
    }
}

/// `{"message": "…"}` — the acknowledgement the publish, yank and unyank paths
/// return for registries whose CLI shows the string back to the user (twine,
/// `gem push`, `terraform login`, `dotnet nuget push`).
///
/// The text is for a human; nothing parses it, and no client should.
#[derive(serde::Serialize, ToSchema)]
pub struct MessageResponse {
    /// Human-readable confirmation of what happened.
    pub message: String,
}

impl MessageResponse {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
