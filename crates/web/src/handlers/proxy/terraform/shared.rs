use super::{
    dispatch_notification, proxy_document, require_local_mode, require_registry_type, web,
    AppError, Arc, AuthIdentity, HttpResponse, LocalRegistryService, NotificationEventType,
    NotificationService, PackageId, ProxyService, RegistryMap, RegistryMode, RegistryModeMap,
};
use actix_web::HttpRequest;
use batlehub_core::entities::Identity;
use batlehub_core::services::{SignedUrlCoordinate, SignedUrlService, SIGNED_URL_QUERY_PARAM};

use crate::handlers::schemas::MessageResponse;

// ── Signed download URLs (RFC 0012) ───────────────────────────────────────────

/// The signer for `registry`, or `None` when this registry does not sign.
///
/// One read-lock snapshot answers both halves of the question — is this
/// registry configured to sign, and with which key. Reading them under two
/// separate locks would let a handler observe a registry switched on by a
/// reload while still holding the signer from before it.
pub async fn signer_for(svc: &ProxyService, registry: &str) -> Option<Arc<SignedUrlService>> {
    let hot = svc.hot.read().await;
    if !hot.signed_downloads.get(registry).copied().unwrap_or(false) {
        return None;
    }
    hot.signed_url.clone()
}

/// Append a minted signature to a provider-artifact URL.
///
/// The caller must already have authenticated *and* authorised `identity` for
/// this coordinate — the signature records that verdict, it does not create one
/// (RFC 0012 §5). There is deliberately no way to mint from a route that has
/// not just done so.
pub fn sign_artifact_url(
    signer: &SignedUrlService,
    url: &str,
    registry: &str,
    package: &str,
    version: &str,
    artifact: &str,
    identity: &Identity,
) -> String {
    let token = signer.mint(
        &SignedUrlCoordinate {
            method: "GET",
            registry,
            package,
            version,
            artifact,
        },
        identity,
    );
    // These URLs are built here and carry no query today. The separator check
    // costs a line and stops a later parameter turning a signed URL into one
    // with two `?`, which Terraform would follow and the verifier would reject.
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}{SIGNED_URL_QUERY_PARAM}={token}")
}

/// Whether `url` is served by this registry: same scheme, host and port, and
/// under `base`'s path prefix.
///
/// Structural rather than a prefix match on the string. A bare-origin `base`
/// makes `starts_with` accept any host that merely begins with it, and the
/// hosts that do — `evil.com` for `ev`, or a sibling registered domain — are
/// exactly the ones an attacker registers. Parsing also disposes of the
/// userinfo trick (`https://good.example@evil.example/`), where the authority
/// is `evil.example` and every textual comparison says otherwise.
fn is_on_origin(url: &str, base: &str) -> bool {
    let (Ok(u), Ok(b)) = (reqwest::Url::parse(url), reqwest::Url::parse(base)) else {
        return false;
    };
    if u.scheme() != b.scheme()
        || u.host_str() != b.host_str()
        || u.port_or_known_default() != b.port_or_known_default()
    {
        return false;
    }
    // A path-routed base carries a prefix (`/proxy/{registry}`) and a
    // host-routed one is just `/`; the empty case admits any path on the
    // origin, which is what being bound to the host root means.
    let prefix = b.path().trim_end_matches('/');
    prefix.is_empty() || u.path() == prefix || u.path().starts_with(&format!("{prefix}/"))
}

/// The checksum URLs in a provider manifest that are **not** on this registry's
/// origin, rendered `field = url` for a message.
///
/// A publisher's `platforms[]` entry carries `shasums_url` and
/// `shasums_signature_url` through to the download document verbatim
/// (`local_registry/eco_terraform.rs`), and Terraform fetches whatever they
/// name. Pointing them at another host is legitimate — today it is the *only*
/// way a local-mode provider install verifies anything, because BatleHub has no
/// key it could put in `signing_keys` — but it has consequences the publisher
/// should be told about rather than discover: that host sees every
/// `terraform init` for this provider, and an air-gapped install reaches it.
///
/// So this is a warning at publish time, not a refusal. Refusing would break the
/// only configuration that works; staying silent leaves the one thing signing
/// cannot cover invisible.
pub fn off_origin_checksum_urls(manifest: &serde_json::Value, base: &str) -> Vec<String> {
    let Some(platforms) = manifest.get("platforms").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = platforms
        .iter()
        .flat_map(|platform| {
            ["shasums_url", "shasums_signature_url"]
                .into_iter()
                .filter_map(move |field| {
                    let url = platform.get(field)?.as_str()?;
                    (!is_on_origin(url, base)).then(|| format!("{field} = {url}"))
                })
        })
        .collect();
    // One line per distinct destination: a manifest with eight platforms all
    // naming the same host should say so once.
    out.sort();
    out.dedup();
    out
}

/// Sign the three URLs of a provider *download document* (RFC 0012 §6.5).
///
/// The registry protocol's document names three absolute URLs on this host, and
/// all three were measured arriving without credentials (§11). Each is signed
/// for its own coordinate, so one cannot be edited into another — and this is
/// the minting site for `required_providers`, which is how providers are
/// actually declared.
///
/// `base` is this registry's public root, and a URL that is not on it is **left
/// alone** — never signed. That is not belt and braces: in local and hybrid
/// mode the document is built from the publisher's own `platforms[]` entry
/// (`local_registry/eco_terraform.rs`), which overwrites `download_url` but
/// carries every other key through verbatim. `shasums_url` and
/// `shasums_signature_url` are therefore **attacker-controlled** by anyone who
/// can publish a provider, and signing one would hand a third party a token
/// minted for whichever user ran `terraform init`.
///
/// The comparison is structural for the same reason. It was `starts_with`
/// until a security review pointed out that `registry_public_base` returns a
/// **bare origin** (`https://tf.acme.io`) when the request is host-routed —
/// which the Terraform registry protocol requires — and a URL's authority ends
/// at `/`, `?`, `#` or `@`. So `https://tf.acme.io.attacker.example/x`,
/// `https://tf.acme.io-evil.net/x` and `https://tf.acme.io@attacker.example/x`
/// all passed a prefix match against it.
pub async fn sign_download_document(
    svc: &ProxyService,
    doc: &mut serde_json::Value,
    coords: DownloadCoords<'_>,
    base: &str,
    identity: &Identity,
) {
    let Some(signer) = signer_for(svc, coords.registry).await else {
        return;
    };
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    for (field, artifact) in [
        ("download_url", format!("{}/{}", coords.os, coords.arch)),
        ("shasums_url", "shasums".to_owned()),
        ("shasums_signature_url", "shasums.sig".to_owned()),
    ] {
        let Some(url) = obj.get(field).and_then(|v| v.as_str()) else {
            continue;
        };
        if !is_on_origin(url, base) {
            // `warn`, not `debug`: reaching here means a document this server
            // is about to serve names a host it does not control, which for the
            // local path means a publisher put it there.
            tracing::warn!(
                field,
                registry = coords.registry,
                package = coords.package,
                "download URL is not on this registry's origin; left unsigned"
            );
            continue;
        }
        let signed = sign_artifact_url(
            &signer,
            url,
            coords.registry,
            coords.package,
            coords.version,
            &artifact,
            identity,
        );
        obj.insert(field.to_owned(), serde_json::Value::String(signed));
    }
}

/// What [`sign_download_document`] needs to name a coordinate, grouped so the
/// function does not take eight positional strings.
#[derive(Clone, Copy)]
pub struct DownloadCoords<'a> {
    pub registry: &'a str,
    pub package: &'a str,
    pub version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
}

/// The `bh_sig` value from a request's query string, if it carries one.
///
/// Hand-parsed rather than through a `web::Query<T>` extractor because these
/// routes must keep accepting whatever else a client appends: an extractor that
/// failed to deserialise would turn an unrelated query parameter into a `400`
/// on the download path.
fn signed_url_token(req: &HttpRequest) -> Option<String> {
    form_urlencoded::parse(req.query_string().as_bytes())
        .find(|(k, _)| k == SIGNED_URL_QUERY_PARAM)
        .map(|(_, v)| v.into_owned())
}

/// The identity to authorise an artifact read as (RFC 0012 §5).
///
/// Three outcomes, and the middle one is the whole feature:
///
/// - the registry does not sign, or the request carries no `bh_sig` — the
///   header identity, exactly as before;
/// - a valid signature — the identity it was minted for, which is then handed
///   to the *same* rule chain, quota and audit as any other read (§6.6). This
///   replaces the `Authorization` header and nothing else;
/// - a signature that does not verify — `403`, never a silent fall back to the
///   header. Falling back would answer an expired URL with whatever the
///   anonymous grant allows, which is the wrong error for the operator and, on
///   a closed registry, the wrong error for the client.
pub async fn identity_for_artifact(
    svc: &ProxyService,
    req: &HttpRequest,
    coord: SignedUrlCoordinate<'_>,
    header_identity: AuthIdentity,
) -> Result<Identity, AppError> {
    // Order matters: a `bh_sig` on a registry with signing off is an ignored
    // query parameter, not a refusal (§7). Reading the signer first is what
    // makes that true.
    let Some(signer) = signer_for(svc, coord.registry).await else {
        return Ok(header_identity.0);
    };
    let Some(token) = signed_url_token(req) else {
        return Ok(header_identity.0);
    };

    signer.verify(&token, &coord).map_err(|e| {
        tracing::debug!(
            registry = coord.registry,
            package = coord.package,
            version = coord.version,
            error = %e,
            "signed URL rejected"
        );
        AppError::forbidden(e.to_string()).coded(e.code())
    })
}

/// The data describing a single Terraform yank/unyank request — everything
/// [`terraform_set_yanked`] needs about *what* is being (un)yanked, grouped so
/// the function's other params stay limited to identity/service handles
/// (mirrors `common.rs`'s `LocalOrProxyArtifactOpts` split for the analogous
/// artifact-serving cluster).
pub struct TerraformYankRequest<'a> {
    pub registry: &'a str,
    pub map: &'a RegistryMap,
    pub mode_map: &'a RegistryModeMap,
    pub pkg_name: &'a str,
    pub version: &'a str,
    /// Human-readable identifier used in the response message, e.g.
    /// `"module {namespace}/{name}/{provider}"` or `"provider {namespace}/{ptype}"`.
    pub display_name: &'a str,
    pub yanked: bool,
}

/// Shared yank/unyank flow for Terraform modules and providers: validates the
/// registry/mode, performs the (un)yank, dispatches the notification, and builds
/// the JSON response message.
pub async fn terraform_set_yanked(
    req: TerraformYankRequest<'_>,
    identity: &AuthIdentity,
    local_svc: &Arc<LocalRegistryService>,
    notification_svc: &web::Data<Option<Arc<NotificationService>>>,
) -> Result<HttpResponse, AppError> {
    require_registry_type(req.registry, "terraform", req.map)?;
    require_local_mode(req.registry, req.mode_map)?;

    let actor = identity.0.user_id.clone().unwrap_or_default();
    let (event_type, verb) = if req.yanked {
        local_svc
            .yank(req.registry, req.pkg_name, req.version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageYanked, "yanked")
    } else {
        local_svc
            .unyank(req.registry, req.pkg_name, req.version, &identity.0)
            .await
            .map_err(AppError::from)?;
        (NotificationEventType::PackageUnyanked, "unyanked")
    };

    dispatch_notification(
        notification_svc,
        event_type,
        req.registry,
        req.pkg_name,
        Some(req.version.to_owned()),
        &actor,
    );

    Ok(HttpResponse::Ok().json(MessageResponse::new(format!(
        "{verb} {}@{}",
        req.display_name, req.version
    ))))
}

/// Shared versions-listing flow for Terraform modules and providers: if `local_result`
/// is `Some`, it's the already-awaited local/hybrid lookup; on `NotFound` in hybrid
/// mode (or `None`, i.e. proxy mode), falls through to streaming the upstream response.
pub async fn terraform_versions_response(
    registry: &str,
    pkg_name: String,
    identity: AuthIdentity,
    svc: web::Data<Arc<ProxyService>>,
    mode: RegistryMode,
    local_result: Option<Result<serde_json::Value, batlehub_core::error::CoreError>>,
) -> Result<HttpResponse, AppError> {
    if let Some(result) = local_result {
        match result {
            Ok(json) => return Ok(HttpResponse::Ok().json(json)),
            Err(batlehub_core::error::CoreError::NotFound(_)) if mode == RegistryMode::Hybrid => {}
            Err(batlehub_core::error::CoreError::NotFound(msg)) => {
                return Err(AppError::not_found(msg))
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }

    // A version listing, not an artifact: `proxy_document` fetches and filters
    // it so `terraform init` never selects a blocked module or provider version
    // and then be refused the download of it.
    let pkg = PackageId::new(registry, pkg_name, "versions");
    proxy_document(
        svc,
        pkg,
        identity,
        batlehub_core::rules::resource_type::RELEASES_READ,
        batlehub_core::ports::DocumentKind::Versions,
        String::new(),
    )
    .await
}

#[cfg(test)]
mod origin_tests {
    use super::is_on_origin;

    /// The bypasses a `starts_with` on a bare origin accepted. Each of these is
    /// a host an attacker can register, and each was signed before this check
    /// became structural.
    #[test]
    fn a_host_that_merely_starts_with_the_base_is_not_on_it() {
        let base = "https://tf.acme.io";
        for hostile in [
            "https://tf.acme.io.attacker.example/s",
            "https://tf.acme.io-evil.net/s",
            "https://tf.acme.io@attacker.example/s",
            "https://tf.acme.io:443@attacker.example/s",
            "https://tf.acme.iox/s",
        ] {
            assert!(!is_on_origin(hostile, base), "accepted {hostile}");
        }
    }

    #[test]
    fn the_registrys_own_urls_are_on_it() {
        let base = "https://tf.acme.io";
        for ours in [
            "https://tf.acme.io/v1/providers/acme/x/1.0.0/shasums",
            "https://tf.acme.io/",
            "https://tf.acme.io",
        ] {
            assert!(is_on_origin(ours, base), "rejected {ours}");
        }
    }

    /// Scheme and port are part of the origin: a downgrade to `http` or a
    /// different port is a different server.
    #[test]
    fn scheme_and_port_must_match() {
        let base = "https://tf.acme.io";
        assert!(!is_on_origin("http://tf.acme.io/s", base));
        assert!(!is_on_origin("https://tf.acme.io:8443/s", base));
        // …but the default port spelled out is the same origin.
        assert!(is_on_origin("https://tf.acme.io:443/s", base));
    }

    /// A path-routed base carries a prefix, and a sibling registry on the same
    /// host is not this registry.
    #[test]
    fn a_path_routed_base_confines_to_its_prefix() {
        let base = "https://hub.acme.io/proxy/tf";
        assert!(is_on_origin("https://hub.acme.io/proxy/tf/v1/x", base));
        assert!(is_on_origin("https://hub.acme.io/proxy/tf", base));
        assert!(!is_on_origin("https://hub.acme.io/proxy/other/v1/x", base));
        // The prefix must end at a segment boundary.
        assert!(!is_on_origin(
            "https://hub.acme.io/proxy/tf-evil/v1/x",
            base
        ));
    }

    #[test]
    fn unparseable_input_is_not_on_the_origin() {
        assert!(!is_on_origin("not a url", "https://tf.acme.io"));
        assert!(!is_on_origin("/relative/path", "https://tf.acme.io"));
        assert!(!is_on_origin("https://tf.acme.io/s", "not a url"));
    }
}
