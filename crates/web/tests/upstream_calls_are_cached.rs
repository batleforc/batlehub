//! Every outbound call under `handlers/proxy/` goes through a caching helper.
//!
//! RFC 0009 §4.2. The rule is that an endpoint which calls upstream is a cache
//! and must survive that upstream's loss — and the reason it needs enforcing
//! rather than documenting is that it was violated three times by three
//! different people writing three passthroughs, each of which made a bare
//! `reqwest` call and failed outright the moment its upstream went away.
//!
//! A source scan rather than a runtime assertion, because the failure is
//! *absence*: nothing calls a helper that was never reached for. The next
//! passthrough inherits the three rungs by having nowhere else to go, and this
//! test is what makes "nowhere else" true.
//!
//! ## Why there are two helpers rather than one
//!
//! §4.2 originally said the enforcement would be that one helper is the only
//! `reqwest` caller. That was written without knowing
//! `jetbrains_marketplace/cached_forward.rs` already existed and had done the
//! same job for longer — see §13.3. Both implement cache-first, upstream, then
//! stale-on-error bounded by the registry's `serve_stale_metadata`. Collapsing
//! them is a refactor of a shipped path for tidiness, not for behaviour, so the
//! allowlist has two entries and each is justified here rather than being a
//! place to add a third quietly.

use std::path::{Path, PathBuf};

/// Files permitted to make an outbound request directly. Both own the three
/// rungs of §4.2; anything else must call one of them.
const HELPERS: &[&str] = &[
    // The helper this RFC added. Owns `ProxyService::cached_passthrough`.
    "handlers/proxy/upstream.rs",
    // Predates it, same contract: cache-first, stale-on-error, size ceiling,
    // TTL fallback, and (since RFC 0009) the `serve_stale_metadata` gate.
    "handlers/proxy/jetbrains_marketplace/cached_forward.rs",
];

/// Call shapes that reach the network on a `reqwest::Client`.
///
/// Deliberately syntactic. A cleverer check would need type information this
/// test does not have, and the point is to make the *easy* way to add a
/// passthrough be the correct one — not to defeat a determined author.
const OUTBOUND: &[&str] = &[".send()", "reqwest::get("];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_handler_calls_upstream_without_a_caching_helper() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/handlers/proxy");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(!files.is_empty(), "found no handler sources to scan");

    let mut offenders = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if HELPERS.iter().any(|h| rel.ends_with(h)) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            // Comments describe the rule; they do not break it.
            if trimmed.starts_with("//") {
                continue;
            }
            if OUTBOUND.iter().any(|pat| line.contains(pat)) {
                offenders.push(format!("  {rel}:{}  {}", i + 1, trimmed.trim_end()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these call upstream without going through a caching helper, so they \
         fail outright when that upstream is unreachable (RFC 0009 §4.2).\n\
         Route them through `handlers::proxy::upstream::cached_forward`:\n{}",
        offenders.join("\n")
    );
}

/// The allowlist is only meaningful if its entries exist.
///
/// A renamed helper would otherwise silently turn the check above into a scan
/// that permits nothing — passing loudly while enforcing a rule about a file
/// that is gone.
#[test]
fn every_allowlisted_helper_exists() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for helper in HELPERS {
        assert!(
            src.join(helper).is_file(),
            "allowlisted helper {helper} does not exist — if it moved, update \
             HELPERS; if it went away, delete the entry"
        );
    }
}
