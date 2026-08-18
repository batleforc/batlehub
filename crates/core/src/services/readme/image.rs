//! What may come back when this server fetches a README's image.
//!
//! The allow-list and the DTO live in `core` rather than beside the HTTP client
//! that fills them because three layers need them and none of them is the HTTP
//! client: the endpoint echoes the type, the SVG sanitiser is chosen by it, and
//! the tests fake the whole thing. Keeping the list here also means the decision
//! about *what an image is* sits next to `svg.rs`, which is the decision about
//! what to do with the awkward one.

/// An image a README pointed at, fetched by this server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedImage {
    /// The `Content-Type` to echo, taken from [`IMAGE_TYPES`] rather than from
    /// the response — an upstream that answered `image/png; charset=utf-8` gets
    /// the canonical spelling, and one that answered something not on the list
    /// never got this far.
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

impl FetchedImage {
    pub fn is_svg(&self) -> bool {
        self.content_type == SVG_CONTENT_TYPE
    }
}

pub const SVG_CONTENT_TYPE: &str = "image/svg+xml";

/// Every `Content-Type` an image response may carry.
///
/// `image/svg+xml` is on it, and RFC 0007-bis was drafted saying it would not
/// be. Two-thirds of README images are SVG (§13.2), so excluding them makes
/// `remote_images = "proxy"` refuse the case that motivated it: a badge row that
/// renders as three broken images and a PNG. It is served **sanitised**
/// ([`super::svg`]) and under a `sandbox` CSP, which are §7.2's two independent
/// controls, either sufficient on its own.
///
/// No `image/x-icon`, no `image/bmp`, no `image/tiff`: none appears in a README,
/// and every entry here is a decoder an operator's browser is asked to run on
/// bytes a package author chose.
pub const IMAGE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    SVG_CONTENT_TYPE,
];

/// The canonical spelling of `raw`, if it names an allow-listed image type.
///
/// Compared on the media type alone: a parameter (`; charset=utf-8`) is not part
/// of the decision, and matching the whole header would refuse a perfectly good
/// PNG on a technicality. Case-insensitive, because the header is.
pub fn image_content_type(raw: &str) -> Option<&'static str> {
    let media = raw.split(';').next()?.trim().to_ascii_lowercase();
    IMAGE_TYPES.iter().copied().find(|t| *t == media)
}

/// The response headers an image is served with, beyond its type.
///
/// The CSP is §7.2's first control and the one that does not depend on
/// [`super::svg`] being right: it stops script in **every** mode a browser has,
/// including the top-level navigation a reader performs by opening the image in
/// a new tab, which is the only mode in which an SVG would otherwise execute
/// with this origin. It is applied to every image and not only to SVG — a PNG
/// has nothing to lose by it, and a type-sniffing bug is exactly the case where
/// a policy that depended on the type would be absent when it mattered.
pub const IMAGE_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_media_type_is_matched_without_its_parameters() {
        assert_eq!(image_content_type("image/png"), Some("image/png"));
        assert_eq!(
            image_content_type("image/svg+xml; charset=utf-8"),
            Some("image/svg+xml")
        );
        assert_eq!(image_content_type("  IMAGE/PNG  "), Some("image/png"));
    }

    #[test]
    fn anything_that_is_not_an_allow_listed_image_is_refused() {
        for raw in [
            "text/html",
            "application/octet-stream",
            "image/x-icon",
            "image/bmp",
            "text/html; charset=utf-8",
            "",
            "image/png/../../etc",
        ] {
            assert_eq!(image_content_type(raw), None, "{raw} should be refused");
        }
    }

    /// The echoed type is a `&'static str` from the list, so a handler cannot
    /// accidentally reflect the upstream's own header into a response.
    #[test]
    fn the_echoed_type_is_the_lists_own_string() {
        let matched = image_content_type("Image/JPEG; q=1").unwrap();
        assert!(IMAGE_TYPES.contains(&matched));
        assert_eq!(matched, "image/jpeg");
    }
}
