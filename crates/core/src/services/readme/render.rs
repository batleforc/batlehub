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

use super::image::image_host_allowed;
use super::sanitize::{sanitize_capturing_images, STRIPPED_IMAGE_CLASS};

/// What the renderer needs to know that is not in the source.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub remote_images: RemoteImagePolicy,
    /// Where a proxied image's `src` is rewritten to, built by the caller from
    /// the request's own origin. What gets appended is the image's **index**, so
    /// this ends in a `/` and the result is
    /// `…/{registry}/{name}/{version}/readme-image/{n}` (RFC 0007-bis §5.1).
    ///
    /// `None` means images are stripped whatever the policy says — there is no
    /// configuration in which a README's image reaches out from the reader's
    /// browser. So does a **relative** prefix, which the sanitiser's
    /// `url_relative(Deny)` would otherwise turn into an `<img>` with no `src`
    /// at all: every image invisible and nothing said about it. See
    /// [`proxy_prefix`].
    pub image_proxy_prefix: Option<String>,
    /// Hosts this registry will fetch an image from. Empty means every host,
    /// which is what `proxy` meant before the list existed.
    ///
    /// The decision is per image rather than per registry: a README that badges
    /// from `shields.io` and screenshots from a personal domain gets the badges
    /// and a chip for the screenshot, instead of an all-or-nothing choice
    /// between a beacon and a blank (RFC 0013 §4.2).
    pub image_hosts: Vec<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            remote_images: RemoteImagePolicy::Strip,
            image_proxy_prefix: None,
            image_hosts: Vec::new(),
        }
    }
}

/// Render `source` to sanitised HTML.
pub fn render(source: &str, format: ReadmeFormat, opts: &RenderOptions) -> String {
    render_capturing_images(source, format, opts).0
}

/// [`render`], plus the URLs of the images the proxied rendering rewrote.
///
/// The indices of the returned vector are the indices in the output's `src`
/// attributes, because both come out of one pass. [`image_urls`] is this run for
/// its second half alone, and it is the only way the image endpoint learns what
/// URL an index means (RFC 0007-bis §5.1).
pub fn render_capturing_images(
    source: &str,
    format: ReadmeFormat,
    opts: &RenderOptions,
) -> (String, Vec<String>) {
    let raw = match format {
        ReadmeFormat::Markdown => markdown_to_html(source, opts),
        // Not re-rendered, only filtered — except that its images become chips
        // like a markdown document's do, for the reason in `chip_html_images`.
        ReadmeFormat::Html if strips_images(opts) => chip_html_images(source, &[]),
        // Proxying from named hosts only: an HTML README's images take the same
        // per-image decision a markdown one's do.
        ReadmeFormat::Html if !opts.image_hosts.is_empty() => {
            chip_html_images(source, &opts.image_hosts)
        }
        ReadmeFormat::Html => source.to_owned(),
        // No markup means no images, and an early return keeps that a fact about
        // the format rather than something the sanitiser happens to arrive at.
        ReadmeFormat::Rst | ReadmeFormat::Plain => return (preformatted(source), Vec::new()),
    };
    sanitize_capturing_images(&raw, opts.remote_images, proxy_prefix(opts).as_deref())
}

/// The URLs of the images in `source`, in the order the renderer numbers them.
///
/// Runs the **same** pipeline `render` runs, under the proxy policy, and keeps
/// only what the sanitiser captured. Not a second walk: a second walk is exactly
/// the thing that could disagree with the first, and the index in an incoming
/// request is only meaningful because it cannot (RFC 0007-bis §5.1).
///
/// The prefix here is a placeholder — the caller wants the list, not the HTML —
/// and it is absolute because a relative one would make the sanitiser drop every
/// `src` and capture nothing.
pub fn image_urls(source: &str, format: ReadmeFormat, image_hosts: &[String]) -> Vec<String> {
    render_capturing_images(
        source,
        format,
        &RenderOptions {
            remote_images: RemoteImagePolicy::Proxy,
            image_proxy_prefix: Some(INDEX_PROBE_PREFIX.to_owned()),
            // **The same list the rendering used**, or the numbering drifts:
            // index 2 of a document rendered under an allow-list is not index 2
            // of the same document rendered without one, and the endpoint would
            // fetch a different image from the one the page asked about — the
            // exact disagreement §5.1's single-walk rule exists to prevent.
            image_hosts: image_hosts.to_vec(),
        },
    )
    .1
}

/// A syntactically valid absolute prefix for the render whose HTML is discarded.
///
/// `.invalid` is reserved by RFC 2606 and resolves nowhere, so if this ever
/// escaped into a document it would fail visibly rather than reach a host.
const INDEX_PROBE_PREFIX: &str = "https://readme-image.invalid/";

/// The proxy prefix, or `None` if it is one the sanitiser would silently eat.
///
/// `url_relative(Deny)` applies to the attribute filter's output, so a relative
/// prefix yields `<img>` with no `src`: every image invisible, no error, no
/// warning — the trap RFC 0007 §4.1 refuses `remote_images = "allow"` over.
/// Falling back to `None` charts the images instead, which is the same answer
/// `strip` gives and is a great deal easier to notice.
fn proxy_prefix(opts: &RenderOptions) -> Option<String> {
    let prefix = opts.image_proxy_prefix.as_deref()?;
    if prefix.starts_with("http://") || prefix.starts_with("https://") {
        return Some(prefix.to_owned());
    }
    tracing::warn!(
        prefix,
        "readme: image proxy prefix is not absolute; charting images instead"
    );
    None
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
        strip_images(parser, &[])
    } else if opts.image_hosts.is_empty() {
        parser.collect()
    } else {
        // Proxying, but only from named hosts: the same chip pass, told which
        // images to leave alone. One pass rather than two, so an image can never
        // be both chipped and rewritten.
        strip_images(parser, &opts.image_hosts)
    };
    let mut out = String::with_capacity(source.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    out
}

fn strips_images(opts: &RenderOptions) -> bool {
    opts.remote_images == RemoteImagePolicy::Strip || proxy_prefix(opts).is_none()
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
/// `keep_hosts` empty chips **every** image — the `strip` policy. Non-empty
/// chips only the images whose host is not in it, which is the allow-list case:
/// the kept ones flow on to the sanitiser and are rewritten to this server.
fn strip_images<'a>(parser: Parser<'a>, keep_hosts: &[String]) -> Vec<Event<'a>> {
    let mut out: Vec<Event<'a>> = Vec::new();
    // Images nest: the alt text between `Start(Image)` and `End(Image)` is
    // itself an event stream, and it can contain another image. A depth counter
    // rather than a bool, so a nested one does not end the outer chip early.
    let mut depth = 0usize;
    let mut alt = String::new();
    let mut host: Option<String> = None;
    /* The image's own events, replayed verbatim when its host is allowed.
    Buffered rather than re-parsed: what reaches the sanitiser then is exactly
    what `pulldown-cmark` produced, and the alt text — which is a stream of
    events, not a string — survives intact. */
    let mut buffered: Vec<Event<'a>> = Vec::new();
    let mut keep = false;

    for event in parser {
        match event {
            Event::Start(Tag::Image { ref dest_url, .. }) => {
                if depth == 0 {
                    alt.clear();
                    host = url_host(dest_url);
                    keep = !keep_hosts.is_empty() && image_host_allowed(dest_url, keep_hosts);
                    buffered.clear();
                }
                depth += 1;
                if keep {
                    buffered.push(event);
                }
            }
            Event::End(TagEnd::Image) => {
                depth = depth.saturating_sub(1);
                if keep {
                    buffered.push(event);
                }
                if depth == 0 {
                    if keep {
                        out.append(&mut buffered);
                        keep = false;
                    } else {
                        out.push(Event::Html(CowStr::from(chip(&alt, host.as_deref()))));
                    }
                }
            }
            // Raw HTML inside markdown: `pulldown-cmark` hands it over
            // untouched, so an author who wrote `<img>` or a `<picture>` rather
            // than `![…]()` would otherwise reach the sanitiser as an element
            // that is not in the allow-list, and vanish entirely.
            Event::Html(html) if depth == 0 => out.push(Event::Html(CowStr::from(
                chip_html_images(&html, keep_hosts),
            ))),
            Event::InlineHtml(html) if depth == 0 => out.push(Event::InlineHtml(CowStr::from(
                chip_html_images(&html, keep_hosts),
            ))),
            // Inside an image, everything is alt text — collected for the chip,
            // and buffered too when the image is being kept.
            //
            // Both halves matter, and the second was missing: a kept image whose
            // alt events were dropped is replayed as `<img src="…" alt="">`, and
            // the panel styles a failed image to "read like the chip, showing its
            // alt text" — so an image that could not be fetched showed the reader
            // nothing at all. Unreachable until `remote_image_hosts` was actually
            // passed to the renderer, which is why it survived: with an empty
            // list `keep` is never true and nothing is ever buffered.
            Event::Text(ref text) | Event::Code(ref text) if depth > 0 => {
                alt.push_str(text);
                if keep {
                    buffered.push(event);
                }
            }
            other if depth > 0 => {
                // Any other markup inside alt text (emphasis, a nested link)
                // contributes nothing a *chip* can show, so it is dropped there —
                // emitting it would put stray tags outside the chip. A kept image
                // is replayed verbatim instead: the sanitiser is what decides
                // about that markup, exactly as it would for an unfiltered render.
                if keep {
                    buffered.push(other);
                }
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
fn chip_html_images(html: &str, keep_hosts: &[String]) -> String {
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
        if !keep_hosts.is_empty() && image_host_allowed(&src, keep_hosts) {
            // Left as written, for the sanitiser to rewrite to this server —
            // the same route a markdown image on an allowed host takes.
            out.push_str(tag);
        } else {
            let alt = tag_attribute(tag, "alt").unwrap_or_default();
            out.push_str(&chip(&alt, url_host(&src).as_deref()));
        }
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
///
/// **This function is a security boundary, not a label.** It is what
/// [`image_host_allowed`] decides `remote_image_hosts` on, in both places it is
/// checked — the render pass and `ReadmeService::image_at`'s re-check — while
/// the *fetch* dials whatever `reqwest::Url` reads. So it has to agree with a
/// WHATWG parser about where the authority ends, or an operator's allow-list can
/// be read past:
///
/// - `\` terminates the authority exactly like `/` for special schemes (WHATWG
///   URL §4.4, and `url`'s parser breaks on it in both the userinfo and host
///   states). Splitting only on `/` read `https://evil.test\@img.shields.io/x`
///   as the host `img.shields.io` — allow-listed — where `reqwest` reads
///   `evil.test` and fetches from it. `ensure_public_url` still refuses internal
///   targets, so this was a policy bypass rather than full SSRF; it defeated the
///   allow-list completely, and defeated *both* checks identically because both
///   call this.
/// - ASCII tab, LF and CR are removed from the URL entirely before parsing, so
///   they cannot be used to hide the difference either.
pub(super) fn url_host(url: &str) -> Option<String> {
    let url: String = url
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest
        .split(['/', '\\', '?', '#'])
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
            // `'` too, and not for symmetry: `decode_basic_entities` turns
            // `&apos;`/`&#39;` into a real apostrophe, so an `alt` written as
            // `&apos; onmouseover=x &apos;` came back out raw and closed the
            // enclosing single-quoted attribute when this chip was emitted
            // inside one. Ammonia drops the injected handler afterwards — but
            // building an injection and relying on the next stage to remove it
            // is exactly what the comment above says this does not do.
            .replace('\'', "&#39;")
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

    fn hosts(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| (*s).to_owned()).collect()
    }

    fn proxying_from(entries: &[&str]) -> RenderOptions {
        RenderOptions {
            remote_images: RemoteImagePolicy::Proxy,
            image_proxy_prefix: Some("https://console.example/img/".to_owned()),
            image_hosts: hosts(entries),
        }
    }

    /// The point of the list: a README that badges from one host and screenshots
    /// from another gets the badges *and* a chip, instead of an all-or-nothing
    /// choice between a beacon and a blank.
    #[test]
    fn an_allowed_host_is_proxied_and_the_rest_are_chipped() {
        let source = "![badge](https://img.shields.io/b.svg)\n\n![shot](https://cdn.example/s.png)";
        let out = render(
            source,
            ReadmeFormat::Markdown,
            &proxying_from(&["shields.io"]),
        );

        assert!(out.contains("https://console.example/img/0"), "{out}");
        assert!(
            !out.contains("img.shields.io"),
            "the badge must be rewritten: {out}"
        );
        assert!(
            out.contains(STRIPPED_IMAGE_CLASS),
            "the screenshot needs a chip: {out}"
        );
        assert!(
            out.contains("cdn.example"),
            "the chip names the host it refused: {out}"
        );
    }

    /// Raw `<img>` inside markdown takes the same decision — an author who
    /// writes HTML must not get a different policy from one who writes markdown.
    #[test]
    fn a_raw_html_image_takes_the_same_decision() {
        let source = "<img src=\"https://img.shields.io/b.svg\" alt=\"b\"> <img src=\"https://cdn.example/s.png\" alt=\"s\">";
        let out = render(
            source,
            ReadmeFormat::Markdown,
            &proxying_from(&["shields.io"]),
        );

        assert!(out.contains("https://console.example/img/0"), "{out}");
        assert!(out.contains("cdn.example"), "{out}");
        assert!(
            !out.contains("https://cdn.example/s.png"),
            "no src for the refused host: {out}"
        );
    }

    /// **The numbering is the endpoint's contract.** `image_urls` must see the
    /// same list the rendering did, or index 1 of the page is index 1 of a
    /// different document.
    #[test]
    fn the_index_list_matches_what_the_page_was_given() {
        let source = "![a](https://cdn.example/a.png)\n\n![b](https://img.shields.io/b.svg)";
        let allowed = hosts(&["shields.io"]);

        let out = render(
            source,
            ReadmeFormat::Markdown,
            &proxying_from(&["shields.io"]),
        );
        let urls = image_urls(source, ReadmeFormat::Markdown, &allowed);

        assert_eq!(urls, vec!["https://img.shields.io/b.svg".to_owned()]);
        assert!(out.contains("https://console.example/img/0"), "{out}");
        assert!(!out.contains("/img/1"), "only one image survived: {out}");
    }

    /// An empty list is every host — what `proxy` meant before the list existed.
    #[test]
    fn an_empty_list_proxies_everything() {
        let source = "![a](https://cdn.example/a.png)\n\n![b](https://img.shields.io/b.svg)";
        let out = render(source, ReadmeFormat::Markdown, &proxying_from(&[]));

        assert!(out.contains("/img/0") && out.contains("/img/1"), "{out}");
        assert!(!out.contains(STRIPPED_IMAGE_CLASS), "{out}");
    }

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
            let out = chip_html_images(tag, &[]);
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
            assert_eq!(chip_html_images(untouched, &[]), untouched);
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
                image_hosts: Vec::new(),
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
                image_hosts: Vec::new(),
            },
        );
        assert!(out.contains("<img"), "{out}");
        assert!(out.contains("hub.example.com"), "{out}");
    }

    // ── The index the image endpoint is addressed by ──────────────────────────

    /// The contract the whole no-URL design rests on: the `n` in a `src` is the
    /// `n` in `image_urls`. Asserted over a document mixing both dialects,
    /// because the two used to be numbered by different code.
    #[test]
    fn image_urls_agree_with_the_indices_the_rendering_emitted() {
        let source = r#"# T

![one](https://a.example/1.png)

<img src="https://b.example/2.png" alt="two">

[![three](https://c.example/3.png)](https://ci.example/job)
"#;
        let (html, urls) = render_capturing_images(
            source,
            ReadmeFormat::Markdown,
            &RenderOptions {
                remote_images: RemoteImagePolicy::Proxy,
                // `.invalid` rather than a host sharing a suffix with the image
                // ones: `hub.example.com` *contains* `b.example`, which made the
                // "no third-party host survives" assertion below fail on output
                // that was perfectly correct.
                image_proxy_prefix: Some("https://hub.invalid/i/".into()),
                image_hosts: Vec::new(),
            },
        );
        assert_eq!(
            urls,
            vec![
                "https://a.example/1.png",
                "https://b.example/2.png",
                "https://c.example/3.png",
            ]
        );
        assert_eq!(urls, image_urls(source, ReadmeFormat::Markdown, &[]));
        for (n, _) in urls.iter().enumerate() {
            assert!(
                html.contains(&format!(r#"src="https://hub.invalid/i/{n}""#)),
                "index {n}: {html}"
            );
        }
        // No third-party host reaches the reader's browser.
        for host in ["a.example", "b.example", "c.example"] {
            assert!(!html.contains(host), "{host}: {html}");
        }
    }

    /// `image_urls` answers the same list whatever the registry's own policy is,
    /// because the endpoint has to resolve an index for a page that was rendered
    /// under `proxy` regardless of what the config says now.
    #[test]
    fn image_urls_answers_for_a_stripped_document_too() {
        let source = "![a](https://a.example/1.png) ![b](https://b.example/2.png)";
        // Rendering under the default policy charts them…
        let charted = md(source);
        assert!(!charted.contains("<img"), "{charted}");
        // …and the list is the same one the proxied rendering would number.
        assert_eq!(
            image_urls(source, ReadmeFormat::Markdown, &[]),
            vec!["https://a.example/1.png", "https://b.example/2.png"]
        );
    }

    #[test]
    fn a_format_with_no_markup_has_no_images() {
        for format in [ReadmeFormat::Rst, ReadmeFormat::Plain] {
            assert!(image_urls("<img src=\"https://a.example/1.png\">", format, &[]).is_empty());
        }
    }

    /// A relative prefix is refused rather than passed to the sanitiser, which
    /// would apply `url_relative(Deny)` to it and leave every `<img>` with no
    /// `src`: images invisible, nothing said, no error. Charting them instead is
    /// the same answer `strip` gives and a great deal easier to notice.
    #[test]
    fn a_relative_proxy_prefix_charts_rather_than_producing_invisible_images() {
        for prefix in ["/api/v1/i/", "i/", "./i/", ""] {
            let out = render(
                "![badge](https://img.shields.io/x.svg)",
                ReadmeFormat::Markdown,
                &RenderOptions {
                    remote_images: RemoteImagePolicy::Proxy,
                    image_proxy_prefix: Some(prefix.into()),
                    image_hosts: Vec::new(),
                },
            );
            assert!(out.contains(STRIPPED_IMAGE_CLASS), "{prefix:?} → {out}");
            assert!(out.contains("img.shields.io"), "{prefix:?} → {out}");
            assert!(!out.contains("<img"), "{prefix:?} → {out}");
        }
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

    /// The authority ends where a WHATWG parser says it does, because
    /// `image_host_allowed` decides an operator's allow-list on this string and
    /// `reqwest` dials the one its own parser reads. A `\` that this treated as
    /// part of the host, and `url` treats as the start of the path, is the whole
    /// bypass: the allow-list saw `img.shields.io`, the fetch went to
    /// `evil.test`.
    #[test]
    fn a_backslash_ends_the_authority_just_as_a_slash_does() {
        assert_eq!(
            url_host(r"https://evil.test\@img.shields.io/badge.svg").as_deref(),
            Some("evil.test")
        );
        assert_eq!(
            url_host(r"https://evil.test\.shields.io/x.svg").as_deref(),
            Some("evil.test")
        );
        // And the same host is still read when there is no backslash to find.
        assert_eq!(
            url_host("https://evil.test/@img.shields.io/badge.svg").as_deref(),
            Some("evil.test")
        );
    }

    /// Tab, LF and CR are removed from a URL before it is parsed, so they cannot
    /// be used to make this and the fetcher read different hosts either.
    #[test]
    fn ascii_tab_and_newline_are_stripped_before_the_host_is_read() {
        assert_eq!(
            url_host("https://evil.test\t\\@img.shields.io/x.svg").as_deref(),
            Some("evil.test")
        );
        assert_eq!(
            url_host("https://ev\nil.test/x.svg").as_deref(),
            Some("evil.test")
        );
    }

    /// The allow-list itself, on the same inputs: the point of the fix.
    #[test]
    fn the_host_allow_list_is_not_bypassed_by_a_backslash() {
        let allowed = vec!["shields.io".to_owned()];
        assert!(!image_host_allowed(
            r"https://evil.test\@img.shields.io/badge.svg",
            &allowed
        ));
        assert!(!image_host_allowed(
            r"https://evil.test\.shields.io/x.svg",
            &allowed
        ));
        // Still allowed when it really is the host.
        assert!(image_host_allowed(
            "https://img.shields.io/badge.svg",
            &allowed
        ));
    }

    /// Empty input is an empty rendering, not a panic and not a `<pre></pre>`
    /// full of nothing.
    #[test]
    fn empty_source_renders_to_nothing_much() {
        assert!(md("").trim().is_empty());
    }
}
