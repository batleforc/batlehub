//! Turning a README's source into HTML the console can display.
//!
//! Four formats, three behaviours:
//!
//! - **Markdown** is parsed as CommonMark with the GFM extensions readers
//!   actually meet — tables, strikethrough, task lists, autolinks, footnotes —
//!   and the result goes through [`super::sanitize`].
//! - **HTML** skips the parse and goes straight to the sanitiser. A README that
//!   already is HTML is not re-rendered; it is filtered.
//! - **RST** and **Plain** are HTML-escaped into a `<pre>`. reStructuredText is
//!   deliberately not rendered: docutils is the only faithful implementation, and
//!   a partial one renders some documents subtly wrong — which is worse than
//!   plainly showing the source (RFC 0007 §3).
//!
//! There is no path from source to output that skips the sanitiser. That is the
//! invariant the fuzz target asserts directly.

use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};

use crate::entities::ReadmeFormat;
use crate::services::hot_config::RemoteImagePolicy;

use super::sanitize::{sanitize, STRIPPED_IMAGE_CLASS};

/// What the renderer needs to know that is not in the source.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub remote_images: RemoteImagePolicy,
    /// Where a proxied image's `src` is rewritten to, built by the caller from
    /// the request's own base URL. `None` means images are stripped whatever the
    /// policy says — there is no configuration in which a README's image reaches
    /// out from the reader's browser.
    pub image_proxy_prefix: Option<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            remote_images: RemoteImagePolicy::Strip,
            image_proxy_prefix: None,
        }
    }
}

/// Render `source` to sanitised HTML.
pub fn render(source: &str, format: ReadmeFormat, opts: &RenderOptions) -> String {
    let raw = match format {
        ReadmeFormat::Markdown => markdown_to_html(source, opts),
        // Not re-rendered, only filtered — except that its images become chips
        // like a markdown document's do, for the reason in `chip_html_images`.
        ReadmeFormat::Html if strips_images(opts) => chip_html_images(source),
        ReadmeFormat::Html => source.to_owned(),
        ReadmeFormat::Rst | ReadmeFormat::Plain => return preformatted(source),
    };
    sanitize(&raw, opts.remote_images, opts.image_proxy_prefix.as_deref())
}

/// The escaped-source rendering, for the formats this deliberately does not
/// parse.
///
/// Built by escaping rather than by sanitising: there is no markup to allow, and
/// running an allow-list over text that was never HTML would silently delete
/// anything that happened to look like a tag — an RST document discussing
/// `<script>` would lose the word.
fn preformatted(source: &str) -> String {
    let mut out = String::with_capacity(source.len() + 16);
    out.push_str("<pre>");
    for ch in source.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out.push_str("</pre>");
    out
}

/// The GFM extension set. Deliberately explicit: each one is markup a reader
/// meets in a real README, and enabling more than that widens what the sanitiser
/// has to answer for.
fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    // Nothing else. Smart punctuation, heading attributes, math and the rest are
    // markup a README rarely uses and the sanitiser would have to answer for.
    options
}

fn markdown_to_html(source: &str, opts: &RenderOptions) -> String {
    let parser = Parser::new_ext(source, markdown_options());
    let events: Vec<Event> = if strips_images(opts) {
        strip_images(parser)
    } else {
        parser.collect()
    };
    let mut out = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    out
}

fn strips_images(opts: &RenderOptions) -> bool {
    opts.remote_images == RemoteImagePolicy::Strip || opts.image_proxy_prefix.is_none()
}

/// Replace every image with a chip carrying its alt text and its host.
///
/// A README's images normally live on third-party hosts. Rendering them means
/// every console page view sends a request — with a `Referer` — to a host chosen
/// by the package author, announcing that someone inside this network is reading
/// about this package at this moment. For an inward-facing proxy whose reason to
/// exist is partly *not* talking to the public internet on every developer
/// action, that is a regression delivered as a feature (RFC 0007 §7.3).
///
/// The chip is what makes stripping honest rather than lossy: the reader can see
/// that an image was there and where it pointed, which is the whole of what a
/// badge row communicates anyway.
fn strip_images<'a>(parser: Parser<'a>) -> Vec<Event<'a>> {
    let mut out: Vec<Event<'a>> = Vec::new();
    // Images nest: the alt text between `Start(Image)` and `End(Image)` is
    // itself an event stream, and it can contain another image. A depth counter
    // rather than a bool, so a nested one does not end the outer chip early.
    let mut depth = 0usize;
    let mut alt = String::new();
    let mut host: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                if depth == 0 {
                    alt.clear();
                    host = url_host(&dest_url);
                }
                depth += 1;
            }
            Event::End(TagEnd::Image) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    out.push(Event::Html(CowStr::from(chip(&alt, host.as_deref()))));
                }
            }
            // Raw HTML inside markdown: `pulldown-cmark` hands it over
            // untouched, so an author who wrote `<img>` or a `<picture>` rather
            // than `![…]()` would otherwise reach the sanitiser as an element
            // that is not in the allow-list, and vanish entirely.
            Event::Html(html) if depth == 0 => {
                out.push(Event::Html(CowStr::from(chip_html_images(&html))))
            }
            Event::InlineHtml(html) if depth == 0 => {
                out.push(Event::InlineHtml(CowStr::from(chip_html_images(&html))))
            }
            // Inside an image, everything is alt text.
            Event::Text(text) | Event::Code(text) if depth > 0 => alt.push_str(&text),
            other if depth > 0 => {
                // Any other markup inside alt text (emphasis, a nested link)
                // contributes nothing a chip can show. Dropped rather than
                // emitted, which would put stray tags outside the chip.
                let _ = other;
            }
            other => out.push(other),
        }
    }
    out
}

/// Replace every raw-HTML `<img>` in a fragment with the same chip a markdown
/// image gets.
///
/// Without this, an image written as HTML rather than as `![…](…)` renders to
/// **nothing at all**: `img` is not in the allow-list when images are stripped,
/// and being a void element it has no children for the sanitiser to keep. A
/// `<picture>` is worse — `picture` and `source` are not allow-listed either, so
/// the whole element disappears and the reader is not told anything was there.
/// That is RFC 0007 §7.3's promise ("the reader can see that an image was there
/// and where it pointed") silently unkept, which the survey in RFC 0007-bis §13.1
/// put at about 7% of real READMEs.
///
/// `<picture>` and `<source>` need no handling of their own: once the fallback
/// `<img>` inside is a chip, the sanitiser drops the two wrapper tags and keeps
/// their contents, so a `<picture>` degrades to its fallback's chip.
///
/// This is a cosmetic rewrite, not a security boundary — the sanitiser still runs
/// afterwards and still owns that. The failure mode of mis-parsing a tag here is
/// a missing chip, which is exactly the behaviour being fixed, never a tag that
/// survives: anything this pass leaves behind faces the same allow-list it would
/// have faced anyway.
fn chip_html_images(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let Some(start) = find_img_tag(html, i) else {
            out.push_str(&html[i..]);
            return out;
        };
        let Some(end) = tag_end(bytes, start) else {
            // An unterminated tag is not a tag. Left as it is, for the
            // sanitiser's parser to make of what it will.
            out.push_str(&html[i..]);
            return out;
        };
        out.push_str(&html[i..start]);
        let tag = &html[start..end];
        let src = tag_attribute(tag, "src").unwrap_or_default();
        let alt = tag_attribute(tag, "alt").unwrap_or_default();
        out.push_str(&chip(&alt, url_host(&src).as_deref()));
        i = end;
    }
    out
}

/// The offset of the next `<img` that actually opens an `img` tag.
///
/// The name has to be followed by something that ends it, or `<images>` would
/// match.
fn find_img_tag(html: &str, from: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = from;
    while i + 4 <= bytes.len() {
        let at = html[i..].find('<')? + i;
        let rest = &bytes[at..];
        if rest.len() >= 4 && rest[1..4].eq_ignore_ascii_case(b"img") {
            match rest.get(4) {
                None => return None,
                Some(c) if c.is_ascii_whitespace() || *c == b'>' || *c == b'/' => return Some(at),
                Some(_) => {}
            }
        }
        i = at + 1;
    }
    None
}

/// The offset just past the `>` that closes the tag opening at `start`.
///
/// Quote-aware, so a `>` inside an attribute value does not end the tag early.
fn tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (offset, byte) in bytes[start..].iter().enumerate() {
        match (quote, byte) {
            (Some(q), b) if *b == q => quote = None,
            (Some(_), _) => {}
            (None, b'"') | (None, b'\'') => quote = Some(*byte),
            (None, b'>') => return Some(start + offset + 1),
            (None, _) => {}
        }
    }
    None
}

/// One attribute's value out of a tag's source text, case-insensitively.
///
/// Handles the three forms an HTML attribute value takes — double-quoted,
/// single-quoted and bare — and decodes the handful of entities that appear in
/// real alt text. Whatever comes out is escaped again by [`chip`], so this
/// decoding cannot widen anything.
fn tag_attribute(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    // Past `<img`, so the element name is never mistaken for an attribute.
    let mut i = 4.min(bytes.len());
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == name_start {
            i += 1;
            continue;
        }
        let found = &tag[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if bytes.get(i) == Some(&b'=') {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            match bytes.get(i) {
                Some(q @ (b'"' | b'\'')) => {
                    let q = *q;
                    i += 1;
                    let value_start = i;
                    while i < bytes.len() && bytes[i] != q {
                        i += 1;
                    }
                    value.push_str(&tag[value_start..i]);
                    i = (i + 1).min(bytes.len());
                }
                Some(_) => {
                    let value_start = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value.push_str(&tag[value_start..i]);
                }
                None => {}
            }
        }
        if found.eq_ignore_ascii_case(name) {
            return Some(decode_basic_entities(&value));
        }
    }
    None
}

/// The five named entities that actually appear in alt text and URLs.
///
/// Not a general entity decoder, and it does not need to be: the result goes
/// through [`chip`]'s escaping, and an entity left undecoded shows as itself
/// rather than as anything executable.
fn decode_basic_entities(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_owned();
    }
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        // Last, so `&amp;lt;` decodes to `&lt;` and not to `<`.
        .replace("&amp;", "&")
}

/// The host of an absolute URL, for the chip to name.
///
/// Parsed rather than pattern-matched, and `None` for anything that is not an
/// absolute `http(s)` URL — a relative `src` has no host to report, and a
/// `javascript:` one is not a host at all.
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .trim();
    (!host.is_empty()).then(|| host.to_owned())
}

/// The chip's markup.
///
/// Escaped here rather than trusted: the alt text and the host both come from
/// the package. The sanitiser would catch it either way — this markup goes
/// through the same pass as the author's own — but building an injection and
/// relying on the next stage to remove it is not a thing to write on purpose.
fn chip(alt: &str, host: Option<&str>) -> String {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let label = if alt.trim().is_empty() {
        "image".to_owned()
    } else {
        escape(alt.trim())
    };
    match host {
        Some(host) => format!(
            r#"<span class="{STRIPPED_IMAGE_CLASS}" title="{}">{label}</span>"#,
            escape(host)
        ),
        None => format!(r#"<span class="{STRIPPED_IMAGE_CLASS}">{label}</span>"#),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(source: &str) -> String {
        render(source, ReadmeFormat::Markdown, &RenderOptions::default())
    }

    #[test]
    fn commonmark_basics_render() {
        let out = md("# Title\n\nSome **bold** and `code`.\n\n- one\n- two\n");
        assert!(out.contains("<h1"), "{out}");
        assert!(out.contains("<strong>bold</strong>"), "{out}");
        assert!(out.contains("<code>code</code>"), "{out}");
        assert!(out.contains("<li>one</li>"), "{out}");
    }

    #[test]
    fn gfm_tables_strikethrough_and_task_lists_render() {
        let out = md("| a | b |\n| - | - |\n| 1 | 2 |\n");
        assert!(out.contains("<table>"), "{out}");
        assert!(out.contains("<th>a</th>"), "{out}");

        assert!(md("~~gone~~").contains("<del>gone</del>"));

        let tasks = md("- [x] done\n- [ ] todo\n");
        assert!(tasks.contains("<input"), "{tasks}");
        assert!(tasks.contains("disabled"), "{tasks}");
    }

    /// `pulldown-cmark` passes raw HTML through, so markdown is not a way round
    /// the allow-list. There is no path from source to output that skips
    /// `sanitize`.
    #[test]
    fn raw_html_inside_markdown_goes_through_the_same_allow_list() {
        let out = md("Before\n\n<script>alert(1)</script>\n\n<p onclick=\"x\">after</p>\n");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(out.contains("after"), "{out}");
    }

    #[test]
    fn a_javascript_link_written_as_markdown_is_still_dropped() {
        let out = md("[click](javascript:alert(1))");
        assert!(!out.contains("javascript"), "{out}");
        assert!(out.contains("click"), "{out}");
    }

    // ── Images ────────────────────────────────────────────────────────────────

    #[test]
    fn a_markdown_image_becomes_a_chip_naming_its_alt_text_and_host() {
        let out = md("![build status](https://img.shields.io/badge/build-passing.svg)");
        assert!(!out.contains("<img"), "{out}");
        assert!(!out.contains("shields.io/badge"), "{out}");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(out.contains("build status"), "{out}");
        assert!(out.contains("img.shields.io"), "{out}");
    }

    #[test]
    fn an_image_with_no_alt_text_still_says_there_was_one() {
        let out = md("![](https://example.com/x.png)");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(out.contains("image"), "{out}");
        assert!(out.contains("example.com"), "{out}");
    }

    /// A badge row is the common case, and each badge is a link wrapping an
    /// image. The link survives; the image becomes a chip inside it.
    #[test]
    fn a_linked_badge_keeps_its_link_and_charts_its_image() {
        let out = md("[![ci](https://img.shields.io/ci.svg)](https://ci.example.com/job)");
        assert!(out.contains("https://ci.example.com/job"), "{out}");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(!out.contains("<img"), "{out}");
    }

    /// A data URI in a README is megabytes of base64 in a database row, and an
    /// SVG data URI is script. It is charted like any other image, and its
    /// payload never reaches the output.
    #[test]
    fn a_data_uri_image_is_charted_and_its_payload_dropped() {
        let out = md("![x](data:image/svg+xml;base64,PHN2Zz48c2NyaXB0Pg==)");
        assert!(!out.contains("base64"), "{out}");
        assert!(!out.contains("svg"), "{out}");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
    }

    /// The alt text and the host come from the package, so the chip escapes
    /// them — building an injection and relying on the next stage to remove it
    /// is not a thing to write on purpose.
    #[test]
    fn the_chip_escapes_what_the_package_wrote() {
        let out = md(r#"![</span><script>alert(1)</script>](https://example.com/x.png)"#);
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("alert(1)</script>"), "{out}");
    }

    // ── Images written as HTML rather than as markdown ────────────────────────
    //
    // These are the assertions that failed before RFC 0007-bis §13.1: an `<img>`
    // the author wrote as HTML rendered to an empty string — no image, no chip,
    // no alt text — because `img` leaves the allow-list when images are stripped
    // and a void element has no children for the sanitiser to keep.

    #[test]
    fn a_raw_html_image_inside_markdown_becomes_a_chip_too() {
        let out = md(r#"Before <img src="https://img.shields.io/ci.svg" alt="ci"> after"#);
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(out.contains("ci"), "{out}");
        assert!(out.contains("img.shields.io"), "{out}");
        assert!(!out.contains("<img"), "{out}");
        assert!(out.contains("Before") && out.contains("after"), "{out}");
    }

    /// The shape RFC 0007-bis §13.1 measured at ~7% of READMEs, and the one that
    /// used to vanish completely: a `<picture>` degrades to its fallback's chip,
    /// because once the inner `<img>` is a chip the sanitiser drops the two
    /// wrapper tags and keeps their contents.
    #[test]
    fn a_picture_degrades_to_its_fallbacks_chip_rather_than_to_nothing() {
        let source = r#"<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://example.com/dark.png">
  <img src="https://example.com/light.png" alt="the logo">
</picture>"#;
        for format in [ReadmeFormat::Markdown, ReadmeFormat::Html] {
            let out = render(source, format, &RenderOptions::default());
            assert!(out.contains(STRIPPED_IMAGE_CLASS), "{format:?} → {out:?}");
            assert!(out.contains("the logo"), "{format:?} → {out:?}");
            assert!(out.contains("example.com"), "{format:?} → {out:?}");
            // The alternative source is not a second chip: only the fallback the
            // reader would have seen is charted.
            assert_eq!(out.matches(STRIPPED_IMAGE_CLASS).count(), 1, "{out:?}");
            assert!(!out.contains("dark.png"), "{format:?} → {out:?}");
        }
    }

    #[test]
    fn an_html_format_readmes_images_are_charted_not_dropped() {
        let out = render(
            r#"<h1>T</h1><p><img alt='badge' src='https://img.shields.io/x.svg'/></p>"#,
            ReadmeFormat::Html,
            &RenderOptions::default(),
        );
        assert!(out.contains("<h1>T</h1>"), "{out}");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(out.contains("badge"), "{out}");
    }

    /// The chip's escaping is what stands between the rewrite and an injection,
    /// so it is asserted on the raw-HTML path too — the sanitiser would catch it
    /// either way, and relying on that is not a thing to write on purpose.
    #[test]
    fn a_raw_html_images_attributes_are_escaped_into_the_chip() {
        let out =
            md(r#"<img src="https://e.example/x.png" alt="</span><script>alert(1)</script>">"#);
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("alert(1)</script>"), "{out}");
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
    }

    #[test]
    fn attribute_parsing_handles_the_three_quoting_forms_and_odd_tags() {
        // Bare, single-quoted, double-quoted; attribute order irrelevant.
        for tag in [
            r#"<img src=https://e.example/x.png alt=logo>"#,
            r#"<img alt='logo' src='https://e.example/x.png'>"#,
            r#"<IMG ALT="logo" SRC="https://e.example/x.png" />"#,
            // A `>` inside a value must not end the tag early.
            r#"<img alt="a > b" src="https://e.example/x.png">"#,
            // A valueless attribute before the ones that matter.
            r#"<img loading src="https://e.example/x.png" alt="logo">"#,
        ] {
            let out = chip_html_images(tag);
            assert!(out.contains("e.example"), "{tag} → {out}");
            assert!(
                !out.contains("<img") && !out.contains("<IMG"),
                "{tag} → {out}"
            );
        }

        // `<images>` is not an `<img>`, and neither is an unterminated tag.
        for untouched in [
            "<images src=x>",
            "<imgfoo>",
            "text with < and no tag",
            "<img src=x",
        ] {
            assert_eq!(chip_html_images(untouched), untouched);
        }
    }

    #[test]
    fn only_the_entities_that_appear_in_alt_text_are_decoded() {
        assert_eq!(decode_basic_entities("a &amp; b"), "a & b");
        assert_eq!(decode_basic_entities("&lt;tag&gt;"), "<tag>");
        // Decoded once, not twice: `&amp;lt;` is the text `&lt;`.
        assert_eq!(decode_basic_entities("&amp;lt;"), "&lt;");
        assert_eq!(decode_basic_entities("nothing to do"), "nothing to do");
    }

    /// Under `proxy`, a raw-HTML image is left alone for the sanitiser to
    /// rewrite — the chipping pass is only for the policy that strips.
    #[test]
    fn a_raw_html_image_is_not_chipped_when_it_is_being_proxied() {
        let out = render(
            r#"<img src="https://img.shields.io/x.svg" alt="badge">"#,
            ReadmeFormat::Html,
            &RenderOptions {
                remote_images: RemoteImagePolicy::Proxy,
                image_proxy_prefix: Some("https://hub.example.com/api/v1/readme-image/".into()),
            },
        );
        assert!(out.contains("<img"), "{out}");
        assert!(out.contains("hub.example.com"), "{out}");
        assert!(!out.contains(STRIPPED_IMAGE_CLASS), "{out}");
    }

    #[test]
    fn proxied_images_keep_the_img_and_point_at_this_server() {
        let out = render(
            "![badge](https://img.shields.io/x.svg)",
            ReadmeFormat::Markdown,
            &RenderOptions {
                remote_images: RemoteImagePolicy::Proxy,
                image_proxy_prefix: Some("https://hub.example.com/api/v1/readme-image/".into()),
            },
        );
        assert!(out.contains("<img"), "{out}");
        assert!(out.contains("hub.example.com"), "{out}");
    }

    // ── The formats that are shown rather than parsed ─────────────────────────

    #[test]
    fn rst_and_plain_come_back_escaped_inside_a_pre() {
        for format in [ReadmeFormat::Rst, ReadmeFormat::Plain] {
            let out = render(
                "Heading\n=======\n\n<script>alert(1)</script>\n",
                format,
                &RenderOptions::default(),
            );
            assert!(out.starts_with("<pre>"), "{out}");
            assert!(out.ends_with("</pre>"), "{out}");
            assert!(out.contains("&lt;script&gt;"), "{out}");
            assert!(!out.contains("<script"), "{out}");
            // The RST markup is shown, not interpreted.
            assert!(out.contains("Heading\n======="), "{out}");
        }
    }

    /// An RST document discussing `<script>` keeps the word. Escaping rather
    /// than sanitising is what makes that true — an allow-list over text that
    /// was never HTML would silently delete it.
    #[test]
    fn escaped_source_keeps_text_that_merely_looks_like_markup() {
        let out = render(
            "Do not write <script> in your docs.",
            ReadmeFormat::Plain,
            &RenderOptions::default(),
        );
        assert!(out.contains("&lt;script&gt;"), "{out}");
        assert!(out.contains("Do not write"), "{out}");
    }

    #[test]
    fn html_is_sanitised_but_not_re_rendered() {
        let out = render(
            "<h1>Title</h1>\n<p># not a heading</p>\n<script>x</script>",
            ReadmeFormat::Html,
            &RenderOptions::default(),
        );
        assert!(out.contains("<h1>Title</h1>"), "{out}");
        // A `#` in HTML is a `#`, not a markdown heading.
        assert!(out.contains("# not a heading"), "{out}");
        assert!(!out.contains("script"), "{out}");
    }

    // ── Host parsing ──────────────────────────────────────────────────────────

    #[test]
    fn the_chip_names_the_host_and_only_for_absolute_http_urls() {
        assert_eq!(
            url_host("https://img.shields.io/badge/x.svg").as_deref(),
            Some("img.shields.io")
        );
        assert_eq!(
            url_host("http://user:pass@example.com:8080/x").as_deref(),
            Some("example.com:8080")
        );
        assert_eq!(url_host("./local.png"), None);
        assert_eq!(url_host("data:image/png;base64,AAAA"), None);
        assert_eq!(url_host("javascript:alert(1)"), None);
        assert_eq!(url_host("https://"), None);
    }

    /// Empty input is an empty rendering, not a panic and not a `<pre></pre>`
    /// full of nothing.
    #[test]
    fn empty_source_renders_to_nothing_much() {
        assert!(md("").trim().is_empty());
    }
}
