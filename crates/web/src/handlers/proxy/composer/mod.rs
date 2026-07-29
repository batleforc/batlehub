pub mod metadata;
pub mod upload;

pub use metadata::{
    composer_dist, composer_p2_metadata, composer_packages_json, composer_security_advisories,
};
pub use upload::{composer_upload, composer_yank};

/// Extract base URL from the incoming request, owned so the `ConnectionInfo`
/// borrow can be released before any `.await` points.
///
/// Forwarded host/scheme headers are honoured only from a trusted peer — see
/// [`crate::middleware::proxy_trust`].
pub(crate) fn build_base_url(req: &actix_web::HttpRequest) -> String {
    crate::middleware::trusted_base_url(req)
}
