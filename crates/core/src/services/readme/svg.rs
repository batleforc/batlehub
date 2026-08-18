//! An allow-list over SVG, for the two-thirds of README images that are one.
//!
//! RFC 0007-bis was drafted refusing to serve SVG at all, on reasoning that is
//! correct as far as it goes: an SVG is a *document*, it can carry script, and
//! this serves it from the console's own origin. What the draft did not know is
//! that **67 % of the images in real READMEs are SVG** (RFC 0007-bis §13.2) —
//! every shields.io badge, every `opencollective` banner. Refusing them would
//! make `remote_images = "proxy"` render a third of a badge row and look broken,
//! which is RFC 0007 §4.1's `"allow"` trap wearing different clothes.
//!
//! So they are served, behind **two independent controls, either sufficient on
//! its own** (RFC 0007-bis §7.2):
//!
//! - the response carries `Content-Security-Policy: default-src 'none'; …;
//!   sandbox`, which stops script in every mode a browser has, including the
//!   top-level navigation an `<img>` never performs but a reader opening the
//!   image in a new tab does. That control does not depend on this file being
//!   right.
//! - this module, which does not depend on the browser being right.
//!
//! Belt and braces here and nowhere else in the RFC, for the reason RFC 0007
//! §7.1 gives about the HTML sanitiser: this is markup an arbitrary publisher
//! authored, rendered on an origin an administrator is authenticated to.
//!
//! ## Why a re-serialising allow-list rather than a filter
//!
//! Nothing of the input survives except what an allow-list named: the document
//! is parsed with `quick-xml`, and a new one is written from the elements and
//! attributes that are permitted. A filter that deleted the bad parts would have
//! to be right about every encoding of every bad part; this has to be right about
//! the good ones, which is a much shorter list and one that fails safe.
//!
//! `quick-xml` never resolves external entities — it hands unknown ones back
//! untouched — so a hostile document gets no XXE primitive here, which is the
//! same property `crates/core`'s Maven metadata rewriting relies on.

use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

/// Every element an SVG we serve may contain.
///
/// Shapes, paths, text, gradients, and the grouping and clipping they need.
/// Everything else goes, and the ones worth naming because they are the attack:
/// `script` obviously; `foreignObject`, which embeds arbitrary HTML — including
/// a `<script>` — inside an SVG and is the classic way past a naive filter;
/// `use` and `image`, which load an external document; `animate` and its family,
/// which carry `onbegin`; `style`, which can `@import`; and `a`, because a badge
/// does not need to navigate and a link inside an image is a click a reader
/// cannot inspect.
const ALLOWED_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "title",
    "desc",
    "symbol",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "pattern",
    "marker",
];

/// Every attribute those elements may carry.
///
/// Presentation and geometry only. There is no `href`, no `xlink:href`, no
/// `style`, and nothing beginning `on` — and the last is enforced by absence
/// from this list rather than by a prefix test, so an encoding trick has nothing
/// to defeat.
const ALLOWED_ATTRIBUTES: &[&str] = &[
    // Geometry
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "width",
    "height",
    "d",
    "points",
    "dx",
    "dy",
    "transform",
    "viewBox",
    "preserveAspectRatio",
    "offset",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "patternUnits",
    "markerWidth",
    "markerHeight",
    "refX",
    "refY",
    "orient",
    "clipPathUnits",
    "maskUnits",
    "textLength",
    "lengthAdjust",
    // Paint
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-miterlimit",
    "opacity",
    "color",
    "stop-color",
    "stop-opacity",
    "clip-path",
    "mask",
    "shape-rendering",
    // Text
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "letter-spacing",
    "word-spacing",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
    // Structural. `id` stays because gradients and clip paths are referenced by
    // it *from within the same document*; the references themselves
    // (`fill="url(#g)"`) are checked below, and the document is served alone at
    // its own URL, so there is no page whose ids it could clobber.
    "id",
    "version",
    "xmlns",
    "role",
    "aria-label",
];

/// The reason a document was rejected outright rather than filtered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgRejected {
    /// The bytes are not XML this can parse. A truncated or malformed document
    /// is not something to guess at: a parser that recovers is a parser whose
    /// idea of the document may differ from the browser's, which is the whole
    /// mXSS family.
    Malformed(String),
    /// No `<svg>` root. Whatever it is, it is not the image it claimed to be.
    NotSvg,
}

impl std::fmt::Display for SvgRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(e) => write!(f, "not parseable as XML: {e}"),
            Self::NotSvg => write!(f, "no <svg> root element"),
        }
    }
}

/// Rewrite `svg` keeping only what [`ALLOWED_ELEMENTS`] and
/// [`ALLOWED_ATTRIBUTES`] name.
///
/// A disallowed element is dropped **with its subtree**, not unwrapped: unlike
/// HTML, where dropping a `<div>` and keeping its text is the useful behaviour,
/// the contents of a `<script>` or a `<foreignObject>` are the payload. Text
/// content is kept only inside `<text>`/`<tspan>`/`<title>`/`<desc>`, so a
/// badge's label survives and a stray string does not become visible markup.
///
/// Returns the rewritten document, or why it was refused entirely.
pub fn sanitize_svg(svg: &[u8]) -> Result<Vec<u8>, SvgRejected> {
    let mut reader = Reader::from_reader(svg);
    let config = reader.config_mut();
    config.trim_text(false);
    config.check_end_names = false;

    let mut writer = Writer::new(Cursor::new(Vec::new()));
    let mut buf = Vec::new();

    // Depth of the disallowed subtree currently being skipped. A counter rather
    // than a flag so a `<script>` inside a `<foreignObject>` does not end the
    // skip when the inner one closes.
    let mut skipping = 0usize;
    // Elements written, so their ends can be matched. A disallowed element's end
    // must not be emitted even when it arrives after the skip has unwound — a
    // malformed document can do that.
    let mut open: Vec<Vec<u8>> = Vec::new();
    let mut saw_root = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SvgRejected::Malformed(e.to_string())),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                if skipping > 0 || !is_allowed_element(&name) {
                    skipping += 1;
                    continue;
                }
                if name == b"svg" {
                    saw_root = true;
                }
                let cleaned = clean_element(&e, &name);
                open.push(name);
                writer
                    .write_event(Event::Start(cleaned))
                    .map_err(|e| SvgRejected::Malformed(e.to_string()))?;
            }

            Ok(Event::Empty(e)) => {
                let name = local_name(e.name().as_ref());
                if skipping > 0 || !is_allowed_element(&name) {
                    continue;
                }
                if name == b"svg" {
                    saw_root = true;
                }
                let cleaned = clean_element(&e, &name);
                writer
                    .write_event(Event::Empty(cleaned))
                    .map_err(|e| SvgRejected::Malformed(e.to_string()))?;
            }

            Ok(Event::End(e)) => {
                if skipping > 0 {
                    skipping -= 1;
                    continue;
                }
                let name = local_name(e.name().as_ref());
                if open.last().map(|n| n == &name).unwrap_or(false) {
                    open.pop();
                    writer
                        .write_event(Event::End(quick_xml::events::BytesEnd::new(
                            String::from_utf8_lossy(&name).into_owned(),
                        )))
                        .map_err(|e| SvgRejected::Malformed(e.to_string()))?;
                }
            }

            // Text survives only where it is content rather than decoration.
            // `quick-xml` gives it to us escaped as it appeared; writing it back
            // through `Event::Text` re-escapes on output, so a `<` in a label
            // cannot become a tag.
            Ok(Event::Text(t)) => {
                if skipping == 0 && open.last().map(|n| holds_text(n)).unwrap_or(false) {
                    let decoded = t.decode().unwrap_or_default().into_owned();
                    let decoded = strip_forbidden_chars(&decoded);
                    writer
                        .write_event(Event::Text(quick_xml::events::BytesText::new(&decoded)))
                        .map_err(|e| SvgRejected::Malformed(e.to_string()))?;
                }
            }

            // `quick-xml` reports an entity or character reference as its own
            // event rather than resolving it into the surrounding text, so
            // dropping it silently would delete the `<` from a label reading
            // `coverage &lt; 80%`.
            Ok(Event::GeneralRef(r)) => {
                if skipping == 0 && open.last().map(|n| holds_text(n)).unwrap_or(false) {
                    if let Some(resolved) = resolve_reference(&r) {
                        writer
                            .write_event(Event::Text(quick_xml::events::BytesText::new(&resolved)))
                            .map_err(|e| SvgRejected::Malformed(e.to_string()))?;
                    }
                }
            }

            // Everything else is dropped without comment: CDATA (a `<script>`
            // hiding place), comments (which re-parse in some engines — the mXSS
            // family), processing instructions, and the doctype, which is where
            // an entity declaration would live.
            Ok(_) => {}
        }
        buf.clear();
    }

    if !saw_root {
        return Err(SvgRejected::NotSvg);
    }
    Ok(writer.into_inner().into_inner())
}

/// An attribute value as a parser will see it, so what is written is what was
/// checked.
///
/// Two normalisations, both of which a browser performs anyway:
///
/// - the characters XML 1.0 forbids are dropped ([`strip_forbidden_chars`]);
/// - tab, newline and carriage return become a space, which is what XML
///   attribute-value normalisation does on parse.
///
/// Doing it here rather than leaving it to the reader is what makes sanitising a
/// **fixed point** — sanitise twice, get the same bytes — and that property is
/// not cosmetic: it is the invariant that caught a real bypass, where a value
/// was checked in one form and emitted in another (see the call site).
fn normalize_attribute_value(raw: &str) -> String {
    strip_forbidden_chars(raw)
        .chars()
        .map(|c| {
            if matches!(c, '\t' | '\n' | '\r') {
                ' '
            } else {
                c
            }
        })
        .collect()
}

/// Drop the characters XML 1.0 forbids outright.
///
/// A NUL or a stray `\x01` in a text node is legal in a Rust `String` and
/// illegal in XML: a browser handed `image/svg+xml` that does not parse shows a
/// broken image, so copying one through would turn a badge into a visible defect
/// for no gain. Found by the fuzz target, which asserts the output is
/// well-formed rather than merely harmless.
///
/// Tab, newline and carriage return are the three control characters XML does
/// allow, and they are kept — a `<title>` may reasonably span lines.
fn strip_forbidden_chars(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}'))
        .collect()
}

/// The character an `&…;` reference stands for, or `None` to drop it.
///
/// The five predefined XML entities and numeric character references, and
/// **nothing else** — a name a DTD declared resolves to nothing here, which is
/// the same refusal that keeps this free of an XXE primitive. Whatever comes
/// back is written through `BytesText`, which escapes it again, so resolving
/// `&#60;` cannot put a `<` in the output.
fn resolve_reference(reference: &quick_xml::events::BytesRef<'_>) -> Option<String> {
    let name = reference.decode().ok()?;
    match name.as_ref() {
        "lt" => return Some("<".to_owned()),
        "gt" => return Some(">".to_owned()),
        "amp" => return Some("&".to_owned()),
        "apos" => return Some("'".to_owned()),
        "quot" => return Some("\"".to_owned()),
        _ => {}
    }
    let digits = name.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse::<u32>().ok()?,
    };
    // A numeric reference can name a character XML forbids just as a literal
    // byte can — `&#1;` is the same problem spelled differently — so the same
    // rule applies rather than a second one.
    char::from_u32(code)
        .map(|c| strip_forbidden_chars(&c.to_string()))
        .filter(|s| !s.is_empty())
}

/// The element name without its namespace prefix.
///
/// `<svg:script>` is a `script`. Matching on the qualified name would let a
/// prefix declaration rename the whole attack out of the allow-list's sight.
fn local_name(qname: &[u8]) -> Vec<u8> {
    match qname.iter().rposition(|b| *b == b':') {
        Some(i) => qname[i + 1..].to_vec(),
        None => qname.to_vec(),
    }
}

fn is_allowed_element(name: &[u8]) -> bool {
    ALLOWED_ELEMENTS
        .iter()
        .any(|allowed| allowed.as_bytes() == name)
}

fn holds_text(name: &[u8]) -> bool {
    matches!(name, b"text" | b"tspan" | b"title" | b"desc")
}

/// Rebuild `element` with only its allow-listed attributes.
fn clean_element<'a>(element: &BytesStart<'a>, name: &[u8]) -> BytesStart<'a> {
    let mut out = BytesStart::new(String::from_utf8_lossy(name).into_owned());
    for attribute in element.attributes().with_checks(false).flatten() {
        let key = local_name(attribute.key.as_ref());
        if !ALLOWED_ATTRIBUTES
            .iter()
            .any(|allowed| allowed.as_bytes() == key)
        {
            continue;
        }
        // Normalise **before** deciding, not after. Found by the fuzz target,
        // as a failure of idempotence: a value reading `url\u{1}(http://evil/x)`
        // passed `safe_value` — the control character broke up the `url(` it
        // looks for — and then `strip_forbidden_chars` removed the character and
        // wrote a working external reference. The rule is to validate the bytes
        // that will be *emitted*, never the ones that arrived.
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .unwrap_or_default()
            .into_owned();
        let value = normalize_attribute_value(&value);
        if !safe_value(&value) {
            continue;
        }
        out.push_attribute((
            String::from_utf8_lossy(&key).into_owned().as_str(),
            value.as_str(),
        ));
    }
    out
}

/// Refuse an attribute value that reaches outside the document.
///
/// The allow-list already excludes every attribute whose *purpose* is to name a
/// URL. This covers the ones that can carry one anyway: `fill`, `stroke`,
/// `clip-path` and `mask` take a `url(…)` reference, which is legitimate when it
/// points at a `#fragment` in the same document and is a request to a
/// third-party host when it does not. A `javascript:` value is refused for the
/// same reason it is in the HTML sanitiser.
fn safe_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.contains("javascript:") || compact.contains("vbscript:") || compact.contains("data:")
    {
        return false;
    }
    // `url(#local)` is fine; `url(http://…)` and `url(//…)` are not — and the
    // test is applied to **every** `url(` in the value, not just the first.
    // `fill` takes a paint server followed by a fallback, so `url(#g)
    // url(http://…)` puts a second reference behind one that passes; checking
    // only the first left the second to the browser, which is exactly the
    // beacon this module exists to refuse.
    for target in compact.split("url(").skip(1) {
        if !target.trim_start_matches(['\'', '"']).starts_with('#') {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;
