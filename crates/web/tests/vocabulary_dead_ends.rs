//! RFC 0015 §11.5 — **the vocabulary has no dead ends in either direction.**
//!
//! > every verb in the enum is requested by at least one route, and every verb a
//! > route requests is in the enum. A verb nothing asks for is a grant an
//! > operator can write that does nothing; a route asking for a verb nobody can
//! > hold is a route nobody can reach. Both have shipped in this tree before,
//! > which is why it is a test rather than a review item.
//!
//! # The two directions are not equally hard
//!
//! **A route cannot request a verb outside the enum.** `Action` is closed and has
//! no `Other(String)` (`entities/permission.rs`), so there is nothing else to
//! pass — the compiler is the assertion and this file adds nothing to it. §13.3
//! records that half as already holding: *"a route cannot request a verb outside
//! the enum, because there is nothing else to pass."*
//!
//! **The other direction is the one that has been false since phase 1.** §13.3
//! deferred it — *"§11.5's dead-end test cannot pass here, because this phase
//! adds the write verbs without yet using them"* — and it stayed deferred through
//! four more phases while the vocabulary grew from 18 verbs to 31. §13.8 had to
//! name eleven unrequested verbs in prose because nothing checked. This is that
//! check.
//!
//! # Why the scope is every route and not the proxy surface
//!
//! §11.5 is explicit, and the reason is a verb this test would otherwise get
//! wrong: *"`catalogue:browse` is requested by the console's explore routes under
//! `/api/v1/`, and a dead-end check scoped to the proxy surface would report the
//! verb unreachable and be wrong."*
//!
//! # How "requested" is decided
//!
//! By reading the source, which wants justifying. A verb is *requested* where it
//! is passed to the decision function, and that is not visible from the router:
//! `authz_matrix.rs` can ask actix which pattern a request matched, but nothing
//! can ask it which `Action` the handler passed three frames down. Some verbs are
//! not requested in a handler at all — `releases:publish` is requested in
//! `local_registry/publish.rs`, below the web crate entirely.
//!
//! So this scans the trees that *ask* and excludes the ones that *grant*. That is
//! the same shape as `ROUTE_INVENTORY`: a stated mapping, checked in both
//! directions so it cannot rot.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use batlehub_core::entities::Action;

/// Source trees where a verb is **requested** — handed to the engine.
const REQUESTING: &[&str] = &[
    "crates/web/src",
    "crates/core/src/services/local_registry",
    "crates/core/src/services/proxy",
    "cli/src",
];

/// Files inside those trees that **grant** rather than request, and so must not
/// count.
///
/// `translate.rs` names most of the vocabulary while building §10 rule 5's grant
/// maps; counting it would make every verb look requested and turn this file
/// green for exactly the wrong reason.
fn is_granting_side(path: &Path) -> bool {
    let p = path.to_string_lossy();
    p.contains("/entities/") || p.ends_with("translate.rs") || p.ends_with("grants.rs")
}

/// Verbs deliberately requested by nothing, each with the reason.
///
/// **This list is the point of the test, not an escape from it.** Every entry is
/// a decision someone has to defend, the test fails when a verb leaves the enum's
/// requested set without being added here, and `no_stale_exceptions` fails when
/// an entry stops being true — so it cannot quietly become a list of everything.
const DELIBERATELY_UNREQUESTED: &[(&str, &str)] = &[
    // ── the four ecosystem verbs (§4.2) ──────────────────────────────────────
    //
    // These name actions this server does not implement, rather than actions it
    // implements without a gate. §13.1 records npm's `dist-tags` endpoints
    // "declining unconditionally with `501`", and there is no OpenVSX namespace
    // claim, Terraform signing-key registration or JetBrains channel assignment
    // at all. §4.2 introduces them as the vocabulary's extensible tail — *"an
    // ecosystem-specific verb is added as a variant like any other"* — and the
    // variant landing before the feature is the order that section describes.
    //
    // A verb for an unimplemented action is not the failure §11.5 is about: it
    // grants nothing because there is nothing to grant, and the day the action
    // ships the compiler has the verb waiting. The failure is a verb for an
    // action that *is* implemented and ungated, which is what the rest of this
    // list would be if it had any other entries of that kind.
    (
        "npm:dist-tags:write",
        "npm dist-tag moves are not implemented (501)",
    ),
    (
        "openvsx:namespace:claim",
        "namespace claiming is not implemented",
    ),
    (
        "terraform:signing-keys:write",
        "signing-key registration is not implemented",
    ),
    (
        "jetbrains:channel:assign",
        "channel assignment is not implemented",
    ),
    // ── two that are genuinely unfinished, and are not the same thing ────────
    //
    // Both gate an action this server *does* implement, so both are the failure
    // §11.5 describes: a grant an operator can write that does nothing. They are
    // listed rather than fixed here because each is a behaviour change with its
    // own argument, and neither belongs in the commit that adds the check.
    (
        "releases:list",
        "listing routes still request `releases:read`; §10 rule 4 exists because \
         the split does not fall cleanly along today's two verbs, so moving them \
         is a change with its own migration argument",
    ),
    (
        "catalogue:browse",
        "the console's explore routes are still gated by `hot_config`'s legacy \
         access sets; §10 rule 2's conjunction reproduces them exactly, so the \
         verb is correct and simply not yet the thing consulted",
    ),
];

/// Every `.rs` file under `dir`.
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

/// The workspace root, from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/web is two levels below the workspace root")
        .to_path_buf()
}

/// Source with every `#[cfg(test)]` module removed.
///
/// A test module names verbs it is *about* rather than verbs a route requests, so
/// counting one would make this file green by describing itself.
///
/// **By brace matching, not by truncating at the first marker.** The first
/// version did truncate, on the assumption that test modules come last — and
/// `ops/quota.rs` puts its at line 15, above every handler, so the scan read six
/// lines of a file that requests `quota:read` four times and reported the verb as
/// a dead end. The convention was not a rule, and a check that depends on one is
/// a check that is wrong wherever the convention is not followed.
fn without_tests(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(i) = rest.find("#[cfg(test)]") {
        out.push_str(&rest[..i]);
        // Skip to the module body, then past its matching close brace.
        let after = &rest[i..];
        let Some(open) = after.find('{') else {
            break;
        };
        let mut depth = 0usize;
        let mut end = None;
        for (offset, ch) in after[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(e) => rest = &after[e..],
            // Unbalanced: drop the remainder rather than count it.
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Whether this file is test code rather than code a route runs.
///
/// A sibling `tests.rs` carries no `#[cfg(test)]` marker of its own — the
/// attribute is on the `mod` declaration in its parent — so stripping inline test
/// modules does not reach it. `local_registry/tests.rs` names `releases:list`,
/// and without this the scan reported the verb as requested by a route when what
/// requests it is an assertion about a fixture.
fn is_test_file(path: &Path) -> bool {
    let p = path.to_string_lossy();
    p.ends_with("/tests.rs") || p.contains("/tests/")
}

/// Every verb some route asks the engine for.
fn requested_verbs() -> BTreeSet<String> {
    let root = workspace_root();
    let mut files = Vec::new();
    for tree in REQUESTING {
        rust_files(&root.join(tree), &mut files);
    }

    let mut found = BTreeSet::new();
    for path in files {
        if is_granting_side(&path) || is_test_file(&path) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let source = without_tests(&source);
        let source = source.as_str();
        for action in Action::ALL {
            // The Rust spelling, which is how a call site names it.
            let variant = format!("{action:?}");
            if source.contains(&format!("Action::{variant}")) {
                found.insert(action.as_str().to_owned());
            }
        }
    }
    found
}

/// The scan finds something, or every assertion below is vacuous.
///
/// The failure this guards against is not subtle: a wrong `REQUESTING` path, a
/// renamed directory, or a `workspace_root` that walks up one level too few makes
/// `requested_verbs` empty, and an empty set would make
/// `every_verb_is_requested_by_some_route` fail loudly — but would make
/// `no_stale_exceptions` pass silently.
#[test]
fn the_scan_actually_reads_the_tree() {
    let requested = requested_verbs();
    assert!(
        requested.len() > 3,
        "the source scan found {} verbs, which means it is not reading the tree it \
         thinks it is — check REQUESTING against the crate layout",
        requested.len()
    );
    assert!(
        requested.contains("releases:read"),
        "`releases:read` is requested by 76 sites; a scan that misses it is broken"
    );
}

/// §11.5's first direction: **no verb is a dead end.**
///
/// A verb nothing requests is a grant an operator can write, `explain` will
/// report, `config:explain` will print — and that changes nothing about what the
/// server does. §4.2 calls that failure out by name for a *typo'd* verb; a real
/// verb nobody consults is the same silence with a spelling that looks right.
#[test]
fn every_verb_is_requested_by_some_route() {
    let requested = requested_verbs();
    let excepted: BTreeSet<&str> = DELIBERATELY_UNREQUESTED.iter().map(|(v, _)| *v).collect();

    let dead_ends: Vec<&str> = Action::ALL
        .iter()
        .map(|a| a.as_str())
        .filter(|v| !requested.contains(*v) && !excepted.contains(v))
        .collect();

    assert!(
        dead_ends.is_empty(),
        "these verbs are in the vocabulary and requested by no route: {dead_ends:?}\n\
         \n\
         A verb nothing asks for is a grant that does nothing — an operator writes \
         it, `explain` reports it, and the server behaves identically. Either wire \
         it to the route it is for, or add it to DELIBERATELY_UNREQUESTED with the \
         reason it has none."
    );
}

/// …and the exception list cannot rot.
///
/// The other half of §11.1's two-directional gate, applied to this list: an entry
/// that has stopped being true is an entry nobody will revisit, and a list of
/// stale exceptions is how a check becomes decoration. When somebody wires
/// `releases:list`, this fails and makes them delete the excuse.
#[test]
fn no_stale_exceptions() {
    let requested = requested_verbs();
    let stale: Vec<&str> = DELIBERATELY_UNREQUESTED
        .iter()
        .map(|(v, _)| *v)
        .filter(|v| requested.contains(*v))
        .collect();

    assert!(
        stale.is_empty(),
        "these verbs are listed as requested by nothing, and are requested: {stale:?}\n\
         Delete the entry — the exception outlived the reason for it."
    );
}

/// Every exception names a verb that exists.
///
/// A typo here would silently except nothing while looking like it excepted
/// something, which is the same class of failure as a typo'd verb in a config —
/// the one §4.2 built a closed enum to remove.
#[test]
fn every_exception_names_a_real_verb() {
    let vocabulary: BTreeSet<&str> = Action::ALL.iter().map(|a| a.as_str()).collect();
    for (verb, reason) in DELIBERATELY_UNREQUESTED {
        assert!(
            vocabulary.contains(verb),
            "'{verb}' is excepted from the dead-end check and is not in the vocabulary"
        );
        assert!(
            !reason.trim().is_empty(),
            "'{verb}' is excepted without a reason, which is an exception nobody can review"
        );
    }
}

/// The second direction, recorded rather than asserted.
///
/// *"Every verb a route requests is in the enum"* holds because `Action` is a
/// closed enum with no free-text variant — there is nothing else a call site
/// could pass, and the compiler rejects the attempt. This test exists so a reader
/// looking for §11.5's second half finds it answered rather than missing.
#[test]
fn a_route_cannot_request_a_verb_outside_the_vocabulary() {
    // Every requested verb parses back to the variant it names: the scan reads
    // Rust spellings and the vocabulary is wire spellings, so a variant that
    // round-trips through neither would mean the two have drifted.
    for verb in requested_verbs() {
        assert!(
            verb.parse::<Action>().is_ok(),
            "'{verb}' was scanned as requested and is not a member of the enum"
        );
    }
}
