pub mod auth;
pub mod back_office;
pub mod front_office;
pub mod healthz;
pub mod inbound_webhook;
pub mod metrics;
pub mod proxy;

/// Clamp caller-supplied `page`/`per_page` query params before they're used to
/// compute a SQL `offset = page * per_page`. `per_page=0` would make `LIMIT 0`
/// return zero rows while a paired count query still reports a nonzero total;
/// `page` is capped to keep `page * per_page` from overflowing `u64`.
pub(crate) fn clamp_pagination(page: u64, per_page: u64) -> (u64, u64) {
    (page.min(10_000), per_page.clamp(1, 100))
}

/// Sanitise a filename for use in a `Content-Disposition: attachment; filename="..."` value.
///
/// Replaces `"`, `\`, `/`, non-ASCII characters, and ASCII control characters
/// (including CR/LF) with `_` so the resulting string is safe inside a
/// quoted-string token (RFC 7230 §3.2.6) and is a valid single-segment file
/// name for download clients that validate it (e.g. IntelliJ's
/// `PluginDownloader`).
pub(crate) fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '"' | '\\' | '/' => '_',
            c if !c.is_ascii() || c.is_ascii_control() => '_',
            c => c,
        })
        .collect()
}
