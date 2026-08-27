//! Escaping for values interpolated into markup and URLs.
//!
//! Most of what this server emits is JSON, where `serde_json` does the escaping
//! and there is nothing to get wrong. The exceptions are the protocol documents
//! that are genuinely HTML — the PEP 503 Simple index above all — and the README
//! pipeline's own markup. Both interpolate values that came from a publisher, so
//! both need the same, single, obviously-correct escape.
//!
//! It lives here rather than in `readme::render` because the Simple index is
//! built in `local_registry` and must not have to depend on the renderer to be
//! safe.

/// Escape a string for interpolation into HTML text **or** into a quoted
/// attribute value.
///
/// Covers `&`, `<`, `>`, `"` and `'`. The last two are not optional: a value
/// that reaches an `href="…"` closes the attribute with `"`, and one that
/// reaches single-quoted markup closes it with `'` — this is exactly the
/// primitive the readme `chip` already documents.
///
/// This escapes *text*. It does not make a hostile URL safe to use as an
/// `href` — for a path component use [`percent_encode_path_segment`] first, and
/// for the scheme rely on the README sanitiser's allow-list.
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    push_escaped_html(&mut out, s);
    out
}

/// [`escape_html`], appending into an existing buffer.
pub fn push_escaped_html(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
}

/// Percent-encode one path segment of a URL (RFC 3986 unreserved set kept,
/// everything else encoded).
///
/// Deliberately stricter than a path-safe encode: `/`, `?`, `#`, `%` and `+`
/// are all encoded, so a value carrying one of them cannot leave the segment it
/// was written into, truncate the path with a fragment, or reappear as a space
/// after form decoding. The bytes come back verbatim once the router decodes
/// the segment, so a coordinate encoded here still resolves.
pub fn percent_encode_path_segment(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_five_characters_that_matter() {
        assert_eq!(
            escape_html(r#"<a href="x" onload='y'>&</a>"#),
            "&lt;a href=&quot;x&quot; onload=&#39;y&#39;&gt;&amp;&lt;/a&gt;"
        );
    }

    #[test]
    fn escaping_is_not_double_applied_to_its_own_output() {
        // Not idempotence — escaping twice must produce a *different*, still
        // inert string. What matters is that no `<` survives the first pass.
        let once = escape_html("<script>");
        assert!(!once.contains('<'), "{once}");
        assert_eq!(escape_html(&once), "&amp;lt;script&amp;gt;");
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        assert_eq!(
            escape_html("requests-2.28.0.tar.gz"),
            "requests-2.28.0.tar.gz"
        );
    }

    #[test]
    fn percent_encodes_everything_outside_the_unreserved_set() {
        assert_eq!(
            percent_encode_path_segment("a b/c?d#e%f+g"),
            "a%20b%2Fc%3Fd%23e%25f%2Bg"
        );
    }

    #[test]
    fn percent_encoding_keeps_a_normal_wheel_name_readable() {
        assert_eq!(
            percent_encode_path_segment("requests-2.28.0-py3-none-any.whl"),
            "requests-2.28.0-py3-none-any.whl"
        );
    }

    #[test]
    fn percent_encoding_handles_multibyte_utf8_per_byte() {
        assert_eq!(percent_encode_path_segment("é"), "%C3%A9");
    }

    /// The two together are what an `href` needs: the encode removes the
    /// characters that would leave the URL, the escape removes the ones that
    /// would leave the attribute.
    #[test]
    fn encode_then_escape_defuses_an_attribute_break() {
        let hostile = r#"x.tar.gz"><script>alert(1)</script>"#;
        let href = escape_html(&percent_encode_path_segment(hostile));
        assert!(!href.contains('<') && !href.contains('"'), "{href}");
    }
}
