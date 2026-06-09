pub mod metadata;
pub mod upload;

pub use metadata::{composer_dist, composer_p2_metadata, composer_packages_json};
pub use upload::{composer_upload, composer_yank};

/// Extract base URL from the incoming request, owned so the `ConnectionInfo`
/// borrow can be released before any `.await` points.
pub(crate) fn build_base_url(req: &actix_web::HttpRequest) -> String {
    let conn = req.connection_info();
    format!("{}://{}", conn.scheme(), conn.host())
}
