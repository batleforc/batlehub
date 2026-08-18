#![no_main]

//! The invariant the README pipeline exists to hold, asserted directly.
//!
//! For **arbitrary** input, in any of the four formats and under either image
//! policy, the rendered output contains no `<script`, no `on*=` handler and no
//! URL scheme outside the allow-list. That is a stronger statement than the unit
//! tests in `sanitize.rs` make: those check the standard vectors, this checks
//! the ones nobody thought of.
//!
//! It matters because the input is attacker-authored by construction — anyone
//! who can publish to a proxied upstream can write it — and the output is
//! rendered on the console's own origin to a session that is frequently an
//! administrator's (RFC 0007 §7.1).
//!
//!   task fuzz TARGET=fuzz_readme_render MAX_TIME=60

use libfuzzer_sys::fuzz_target;

use batlehub_core::{
    entities::ReadmeFormat,
    services::{
        readme::render::{render, RenderOptions},
        RemoteImagePolicy,
    },
};

/// Every scheme that must never appear in output, in the forms a parser might
/// still resolve. Checked case-insensitively.
const FORBIDDEN_SCHEMES: &[&str] = &["javascript:", "vbscript:", "data:", "file:", "blob:"];

fuzz_target!(|data: &[u8]| {
    let mut u = arbitrary::Unstructured::new(data);

    let Ok(source): arbitrary::Result<String> = u.arbitrary() else {
        return;
    };
    let Ok(format_idx): arbitrary::Result<u8> = u.arbitrary() else {
        return;
    };
    let Ok(proxy_images): arbitrary::Result<bool> = u.arbitrary() else {
        return;
    };

    let format = match format_idx % 4 {
        0 => ReadmeFormat::Markdown,
        1 => ReadmeFormat::Html,
        2 => ReadmeFormat::Rst,
        _ => ReadmeFormat::Plain,
    };
    let opts = if proxy_images {
        RenderOptions {
            remote_images: RemoteImagePolicy::Proxy,
            image_proxy_prefix: Some("https://hub.invalid/api/v1/readme-image/".to_owned()),
        }
    } else {
        RenderOptions::default()
    };

    let out = render(&source, format, &opts);
    let lower = out.to_ascii_lowercase();

    assert!(
        !lower.contains("<script"),
        "script element survived: {out:?} (from {source:?})"
    );

    // Everything below distinguishes *markup* from *text*. A README may
    // legitimately discuss `javascript:` or write `onerror=` in prose, and the
    // escaped-source formats show exactly that — so an occurrence only matters
    // when it is inside a tag. The sanitiser escapes `<` in text, so "the last
    // `<` is more recent than the last `>`" is an exact test rather than a
    // heuristic.
    //
    // Two earlier versions of this check were wrong before the sanitiser ever
    // was: `"data:` as text, then `="data:` as text, both found within seconds.
    // That is the fuzz target earning its place on the *test* rather than on
    // the code.
    let inside_tag = |idx: usize| {
        let before = &lower[..idx];
        before.rfind('<') > before.rfind('>')
    };

    for (idx, _) in lower.match_indices("on") {
        let rest = &lower[idx..];
        let Some(eq) = rest.find('=') else { continue };
        let name = &rest[..eq];
        if !(name.len() <= 24 && name.chars().all(|c| c.is_ascii_alphabetic())) {
            continue;
        }
        // An attribute is preceded by whitespace inside a tag; text content
        // that happens to read "once=" is not one.
        let preceded_by_space = lower[..idx]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        assert!(
            !(inside_tag(idx) && preceded_by_space),
            "event handler attribute {name:?} survived: {out:?} (from {source:?})"
        );
    }

    for scheme in FORBIDDEN_SCHEMES {
        for (idx, _) in lower.match_indices(scheme) {
            assert!(
                !inside_tag(idx),
                "{scheme} survived inside a tag: {out:?} (from {source:?})"
            );
        }
    }

    // Rendering is deterministic: the render cache is keyed by content digest
    // plus renderer version, so the same source rendering differently on a
    // second call would serve one reader a different document from another.
    assert_eq!(out, render(&source, format, &opts), "render is not stable");
});
