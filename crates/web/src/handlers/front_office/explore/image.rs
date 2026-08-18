//! `GET /api/v1/explore/packages/{registry}/{name}/{version}/readme-image/{index}`
//! (RFC 0007-bis §4.2).
//!
//! **The caller supplies no URL.** It names a coordinate and an index, and the
//! server resolves the URL by walking that version's stored README to the *n*th
//! image — the same walk, over the same bytes, that produced the `src` the
//! browser is asking about. That is what stops this being an open image proxy
//! for whatever a package author writes, and it is why there is no signing key
//! and no host allow-list to maintain (§5.1).
//!
//! It inherits every gate the README endpoint applies, by calling the same
//! `resolve_readme`: an `internal` package's images are not a side channel round
//! the gate that hides its name, and a blocked version serves no README and
//! therefore no images.

use actix_web::HttpResponse;

use super::readme::{resolve_readme, ResolveInput, Resolved};
use super::{get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize, IntoParams};
use batlehub_core::services::{
    hot_config::RemoteImagePolicy,
    readme::{image::IMAGE_CSP, ReadmeImageConfig},
    LocalRegistryService, ProxyService,
};

use crate::{RegistryMap, RegistryModeMap};

/// No image at that index, or it could not be got.
///
/// One code for every reason on purpose. The panel's answer is the same in all
/// of them — fall back to the chip `strip` would have shown — and telling a
/// caller *which* upstream refused an image is a fact about a third-party host
/// that this endpoint has no reason to relay.
pub const README_IMAGE_UNAVAILABLE: &str = "readme.image-unavailable";

#[derive(Deserialize, IntoParams)]
pub struct ReadmeImagePath {
    pub registry: String,
    pub name: String,
    pub version: String,
    /// The image's position in this version's README, counted the way the
    /// renderer counts.
    pub index: usize,
}

/// One image from a version's README, fetched by this server.
#[utoipa::path(
    get,
    path = "/api/v1/explore/packages/{registry}/{name}/{version}/readme-image/{index}",
    tag = "explore",
    params(ReadmeImagePath),
    responses(
        (status = 200, description = "The image bytes", body = crate::handlers::schemas::ArtifactBytes,
         content_type = "application/octet-stream"),
        (status = 403, description = "The version is blocked; the body carries the reason"),
        (status = 404, description = "No such image, or the package is not visible to this caller"),
    ),
    security(("bearer_token" = [])),
)]
// Seven extractors, for the same reason `explore_package_readme` has eight: each
// one gates or shapes the answer, and every gate here is the README endpoint's
// own — deliberately, because an image is part of a README.
//
// Notably absent: the request itself. This endpoint generates no URL, so it has
// no need of `trusted_origin` and no way for a forwarded header to influence
// what it does.
#[allow(clippy::too_many_arguments)]
#[get("/api/v1/explore/packages/{registry}/{name}/{version}/readme-image/{index}")]
pub async fn explore_readme_image(
    path: web::Path<ReadmeImagePath>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    proxy_svc: web::Data<Arc<ProxyService>>,
    registry_map: web::Data<RegistryMap>,
    mode_map: web::Data<RegistryModeMap>,
) -> Result<HttpResponse, AppError> {
    let registry = &path.registry;
    let name = &path.name;

    // Read the policy first. Under `strip` no rendering ever emitted a URL
    // pointing here, so a request is either stale markup from before an operator
    // flipped the switch or somebody probing — and neither should cause this
    // server to talk to a third-party host. Flipping to `strip` therefore stops
    // the egress immediately, which is what an operator setting it expects.
    let (policy, image_cfg) = image_config(&local_svc, registry).await;
    if policy != RemoteImagePolicy::Proxy {
        return Err(unavailable(
            "images are charted rather than fetched for this registry",
        ));
    }

    let Resolved { answer, .. } = resolve_readme(ResolveInput {
        registry,
        name,
        version: Some(&path.version),
        identity: &identity,
        admin_svc: &admin_svc,
        local_svc: &local_svc,
        proxy_svc: &proxy_svc,
        registry_map: &registry_map,
        mode_map: &mode_map,
    })
    .await?;

    let Some(readme_svc) = proxy_svc.readme.as_ref() else {
        return Err(unavailable("this instance stores no READMEs"));
    };

    let Some(image) = readme_svc
        .image_at(&answer.readme, path.index, &image_cfg)
        .await
    else {
        // Every miss reads the same from here: no such index, the upstream said
        // no, the type was not an image, it was too big, or the SVG did not
        // survive the sanitiser. The panel falls back to the chip, which is a
        // better answer than a broken-image icon and the reason none of these is
        // an error (§4.2).
        return Err(unavailable("no image at that index"));
    };

    let cache_control = format!("private, max-age={}", image_cfg.ttl.as_secs());
    Ok(HttpResponse::Ok()
        .content_type(image.content_type)
        // §7.2's first control, and the one that does not depend on the SVG
        // sanitiser being right: it stops script in *every* mode a browser has,
        // including the top-level navigation a reader performs by opening the
        // image in a new tab — the one mode in which an SVG served from this
        // origin would otherwise execute with it. Applied to every type, not
        // only to SVG: a PNG loses nothing by it, and a type-sniffing bug is
        // exactly the case where a policy conditioned on the type would be
        // absent when it mattered.
        .insert_header(("Content-Security-Policy", IMAGE_CSP))
        .insert_header(("Content-Disposition", "inline"))
        // `private`, not `public`: the response is behind the visibility gate,
        // so a shared cache must not hold an internal package's badge where the
        // next caller could be someone who may not see that the package exists.
        .insert_header(("Cache-Control", cache_control))
        .body(image.bytes))
}

/// The registry's image policy and caps, snapshotted out of hot config.
///
/// The TTLs are the registry's own rather than a second set of numbers: the
/// bytes ride the metadata cache, for the reason RFC 0007 §4.1 gives about
/// `upstream_detail` — a second, independently clocked expiry for bytes that
/// already have one is how two caches come to disagree.
async fn image_config(
    local_svc: &LocalRegistryService,
    registry: &str,
) -> (RemoteImagePolicy, ReadmeImageConfig) {
    let hot = local_svc.hot.read().await;
    let readme = hot.readme.get(registry).cloned().unwrap_or_default();
    let ttl = hot
        .policies
        .get(registry)
        .and_then(|p| p.metadata_ttl)
        .unwrap_or(DEFAULT_IMAGE_TTL);
    // The discovery read's shape, reused rather than restated: "how long a `no`
    // is remembered" is the same question here, and 3.3 % of real README image
    // URLs are dead (RFC 0007-bis §13.2), so this is a bounded, measured saving
    // rather than a hypothetical one.
    let negative_ttl = hot
        .upstream_detail
        .get(registry)
        .map(|d| d.negative_ttl)
        .unwrap_or_else(|| {
            std::time::Duration::from_secs(
                batlehub_core::services::DEFAULT_UPSTREAM_NEGATIVE_TTL_SECS,
            )
        });
    (
        readme.remote_images,
        ReadmeImageConfig {
            max_bytes: readme.image_max_bytes,
            ttl,
            negative_ttl,
        },
    )
}

/// Used when a registry sets no `metadata_ttl_secs` of its own.
///
/// An hour rather than the metadata default, because a badge changes far less
/// often than a package listing and every miss is an outbound request.
const DEFAULT_IMAGE_TTL: std::time::Duration = std::time::Duration::from_secs(3600);

/// The single answer for every way an image can fail to be there.
fn unavailable(why: &str) -> AppError {
    AppError::not_found(why.to_owned()).coded(README_IMAGE_UNAVAILABLE)
}
