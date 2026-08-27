//! The route-by-route authorization matrix.
//!
//! # Why this file exists
//!
//! The 2026-08-26 security survey (`docs/internal/security-survey-2026-08-26.md`)
//! found the *same* defect in eight places: a local-registry read path that
//! serves package data without evaluating the registry rule chain, or without
//! checking per-package visibility, while the equivalent proxy-mode read
//! enforces both. It was found one ecosystem at a time — maven, nuget,
//! terraform twice, conda, goproxy, jetbrains, pypi — because the check is
//! applied **by convention rather than by construction**. It had already been
//! found and fixed once before that, on the OpenVSX route
//! (`handlers/proxy/openvsx.rs`), and came back.
//!
//! A per-handler code review cannot close this class, and neither can static
//! analysis: a handler with a guarded proxy branch and an unguarded local branch
//! *mentions* `authorize_read` either way, so "does this function call the
//! chain?" answers yes for exactly the routes that are broken. The question is
//! per-path, not per-function, and the only reliable way to ask it is to make
//! the request and look at the status code.
//!
//! # What it asserts
//!
//! For each row: a registry in local mode whose `[registries.rbac]` grants
//! **anonymous nothing**, holding one published package at the default
//! `Visibility::Public`. An anonymous request to the route must not be answered
//! with `200`. The package is public, so only the rule chain can refuse — which
//! is the point.
//!
//! Every row is paired with a positive control: the identical request as
//! `USER_TOKEN`, which the policy *does* grant, must return `200`. Without it a
//! row could pass because the route 404s for an unrelated reason — a seeding
//! mistake, a path typo — and assert nothing at all. A row whose control fails
//! is a broken test, not a passing gate, and is reported as such.
//!
//! # Known gaps
//!
//! Rows for defects the survey found and that are not yet fixed are marked
//! [`Expect::KnownGap`]. They are held to the *current* behaviour, so the suite
//! is green on a tree with open findings — but the ratchet works in both
//! directions: fixing a handler without flipping its row fails this file, and so
//! does a row marked `Denied` that regresses. Deleting the row is the only way
//! to make a gap disappear quietly, and that shows up in review.
//!
//! # Adding a registry type
//!
//! Two steps, and the second is enforced.
//!
//! 1. Add rows to [`matrix`] — one per read route, with both axes. A row is
//!    `Row::new(kind, uri)`, which expects both gates to refuse and seeds
//!    `pkg` / `9.8.7`; chain `.pkg()`, `.coord()`, `.meta()`, `.vis()` or
//!    `.no_control()` for whatever is *not* true of your route, and nothing
//!    else. What is written beside a row is what is unusual about it.
//! 2. Add every one of the new routes to [`ROUTE_INVENTORY`], classified.
//!
//! Step 2 is not optional and not a convention: `the_route_inventory_matches_the_router`
//! asserts the inventory against the live route table in both directions, so a
//! route this server registers and nobody has classified fails the build, and the
//! failure prints the line to paste. That check exists because this file's own
//! guidance used to end at step 1 — "not covered until someone does" — which is
//! the same *by convention rather than by construction* failure the matrix was
//! written to end, one level up. The ninth ecosystem would have arrived with no
//! rows and a green suite.
//!
//! Coverage is not all-or-nothing: [`Coverage::NoRow`] records a package read
//! nobody has exercised yet, so the list is honest while it shrinks.
//! `report_read_surface_coverage` prints the ratio on every run, because a green
//! suite covering 43 of 97 routes otherwise looks exactly like one covering all
//! of them.
//!
//! `task authz-matrix` prints the table.

mod common;
#[allow(unused_imports)]
use common::*;

use std::sync::Arc;

use actix_web::test::{call_service, TestRequest};
use bytes::Bytes;
use serde_json::json;
use sha2::{Digest, Sha256};

use batlehub_config::schema::RegistryMode;
use batlehub_core::entities::{Identity, Role};
use batlehub_core::ports::TeamNamespacePort;
use batlehub_core::services::PublishRequest;

/// What the matrix expects a request from a caller the axis denies.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Expect {
    /// The gate refuses. `403`, or `404` where the protocol hides existence
    /// rather than admitting it.
    Denied,
    /// A known, unfixed defect: the route answers `200` to a caller the gate
    /// denies. Pinned so the suite is green with the finding open, and so fixing
    /// it forces this row to be updated.
    ///
    /// **No row uses it today** — every gap the 2026-08-26 survey found is
    /// closed — which is why it needs the `allow`. Kept rather than deleted
    /// because it is the ratchet's mechanism, and the next finding wants to be
    /// pinnable on the day it is found rather than after a debate about how to
    /// keep the suite green.
    #[allow(dead_code)]
    KnownGap(&'static str),
    /// This axis is not applicable to this row, or the fixture cannot express
    /// it. Stated rather than silently omitted.
    NotChecked(&'static str),
}

/// One route under test.
struct Row {
    /// Registry type, as `RegistryMap` knows it.
    kind: &'static str,
    /// Route to request, with `{registry}` already substituted.
    uri: &'static str,
    /// **Axis A — the registry rule chain.** Registry RBAC grants anonymous
    /// nothing; the package is `Visibility::Public`. Only the chain can refuse.
    expect: Expect,
    /// **Axis B — per-package visibility.** Registry RBAC grants anonymous
    /// `releases:read` *and* `source:read`, so the chain allows; the package is
    /// `Visibility::Internal`, which `check_visibility` refuses below `User`.
    /// Only visibility can refuse.
    expect_vis: Expect,
    /// Extra index metadata the ecosystem's read path needs to find the package.
    meta: fn() -> serde_json::Value,
    /// Package coordinate to seed.
    name: &'static str,
    version: &'static str,
    /// Skip the positive control: a few routes legitimately 404 even for a
    /// permitted caller in this minimal fixture (they need multi-file artifacts
    /// or an upstream). Stated per row rather than globally.
    control: bool,
}

/// Axis B is only meaningful on routes that serve *this package's* data.
/// Whole-registry documents (`/versions`, `/names`, a search index) are scoped
/// to the channel, and per-package visibility is not the gate that governs them.
const WHOLE_REGISTRY: Expect =
    Expect::NotChecked("whole-registry document; per-package visibility is not its gate");

fn no_meta() -> serde_json::Value {
    json!({})
}

/// The bytes the fixture publishes, so an artifact leak is recognisable.
const FIXTURE_BYTES: &[u8] = b"matrix-fixture-bytes";

/// Did this response disclose the package to a caller the gate denies?
///
/// Status alone is the wrong test. A search or whole-registry index answering
/// `200` with an **empty** result set has disclosed nothing and is behaving
/// correctly; an artifact route answering `200` with the bytes has disclosed
/// everything. So the question is not "was it a 200" but "did the package
/// appear in it" — which is also the property the operator actually cares
/// about, and the one a status-only assertion gets wrong in both directions.
async fn disclosed<S>(app: &S, uri: &str, pkg: &str, version: &str) -> (bool, u16)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    disclosed_as(app, uri, pkg, version, None).await
}

/// [`disclosed`], as a specific caller. `None` is anonymous.
async fn disclosed_as<S>(
    app: &S,
    uri: &str,
    pkg: &str,
    version: &str,
    token: Option<&str>,
) -> (bool, u16)
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<actix_web::body::BoxBody>,
        Error = actix_web::Error,
    >,
{
    let mut req = TestRequest::get().uri(uri);
    if let Some(t) = token {
        req = req.insert_header(("Authorization", bearer(t)));
    }
    // Both the request and the body read are bounded.
    //
    // Several rows are routes that fall through to the upstream client, and the
    // fixture's upstream can leave a request outstanding indefinitely — an
    // unbounded `call_service` or `read_body` hangs the entire suite rather than
    // failing one row, which is a far worse failure mode than a wrong answer.
    //
    // A request that never answers has disclosed nothing, so a timeout resolves
    // to "not disclosed" and reports status `0`. That cannot silently pass a
    // row: the row's positive control makes the same request, times out the
    // same way, discloses nothing, and is reported as a broken control.
    let Ok(resp) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        call_service(app, req.to_request()),
    )
    .await
    else {
        return (false, 0);
    };
    let status = resp.status().as_u16();
    if status != 200 {
        return (false, status);
    }
    let body = match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        actix_web::test::read_body(resp),
    )
    .await
    {
        Ok(b) => b,
        Err(_) => return (false, status),
    };
    let text = String::from_utf8_lossy(&body);
    // Three signals, because a document can disclose the package without naming
    // it: `/info/{gem}` and `@v/list` are keyed by the URL and their bodies are
    // version lists, so the version string is the only thing that betrays them.
    // `9.8.7` is deliberately unusual so `contains` cannot collide with an
    // unrelated version elsewhere in a document.
    let leaked = body
        .windows(FIXTURE_BYTES.len())
        .any(|w| w == FIXTURE_BYTES)
        || text.contains(pkg)
        || text.contains(version);
    (leaked, status)
}

impl Row {
    /// A row with the defaults every route shares: both axes expect the gate to
    /// refuse, no extra index metadata, the coordinate `pkg` / `9.8.7`, and the
    /// positive control on. Only a row's *exceptions* are spelled out at the
    /// call site, so what is unusual about a route is the only thing written
    /// next to it.
    fn new(kind: &'static str, uri: &'static str) -> Self {
        Row {
            kind,
            uri,
            expect: Expect::Denied,
            expect_vis: Expect::Denied,
            meta: no_meta,
            name: "pkg",
            version: "9.8.7",
            control: true,
        }
    }

    /// Override **axis A**, the registry rule chain.
    ///
    /// No row uses it today — every route is expected to refuse — and it is kept
    /// for the same reason as [`Expect::KnownGap`], which is the only thing it
    /// would carry: the next finding wants to be pinnable on the day it is
    /// found, not after a debate about how to keep the suite green.
    #[allow(dead_code)]
    fn chain(mut self, expect: Expect) -> Self {
        self.expect = expect;
        self
    }

    /// Override **axis B**, per-package visibility.
    fn vis(mut self, expect_vis: Expect) -> Self {
        self.expect_vis = expect_vis;
        self
    }

    /// Extra index metadata this ecosystem's read path needs to find the package.
    fn meta(mut self, meta: fn() -> serde_json::Value) -> Self {
        self.meta = meta;
        self
    }

    /// Seed under a different package name. The version stays `9.8.7`.
    fn pkg(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Seed under a different name *and* version.
    fn coord(mut self, name: &'static str, version: &'static str) -> Self {
        self.name = name;
        self.version = version;
        self
    }

    /// Skip the positive control: this route legitimately 404s even for a
    /// permitted caller in the minimal fixture. Every use says why.
    fn no_control(mut self) -> Self {
        self.control = false;
        self
    }
}

/// The cargo sparse-index line, which both cargo rows read.
fn cargo_index_meta() -> serde_json::Value {
    json!({"name": "pkg", "vers": "9.8.7", "deps": [], "cksum": "", "features": {}, "yanked": false})
}

/// Terraform provider coordinates, plus the one platform the download row asks for.
fn terraform_provider_meta() -> serde_json::Value {
    json!({"kind": "provider", "namespace": "acme", "type": "vault", "platforms": [{"os": "linux", "arch": "amd64"}]})
}

/// Terraform module coordinates.
fn terraform_module_meta() -> serde_json::Value {
    json!({"kind": "module", "namespace": "acme", "name": "vpc", "provider": "aws"})
}

/// The sdist filename the PyPI read paths key on.
fn pypi_meta() -> serde_json::Value {
    json!({"filename": "pkg-9.8.7.tar.gz"})
}

/// The conda package filename and the subdir channel holding it.
fn conda_meta() -> serde_json::Value {
    json!({"filename": "pkg-9.8.7-py311_0.conda", "subdir": "linux-64"})
}

/// A marketplace extension's identity, for the openvsx and VS Code rows.
fn extension_meta() -> serde_json::Value {
    json!({"id": "acme.ext", "version": "9.8.7"})
}

/// A JetBrains plugin's identity.
fn plugin_meta() -> serde_json::Value {
    json!({"id": "org.acme.plugin", "version": "9.8.7"})
}

/// `get_go_version_list` reads each version from `index_metadata["Version"]`, so
/// with `no_meta` the list came back *empty for everyone* — including the
/// permitted caller, which is what the broken positive control was reporting. A
/// row whose control cannot see the package asserts nothing about the caller who
/// should not.
fn go_list_meta() -> serde_json::Value {
    json!({"Version": "v9.8.7"})
}

fn matrix() -> Vec<Row> {
    vec![
        // ── cargo ────────────────────────────────────────────────────────────
        Row::new("cargo", "/proxy/reg/pkg/9.8.7/download").meta(cargo_index_meta),
        // ── npm ──────────────────────────────────────────────────────────────
        Row::new("npm", "/proxy/reg/pkg/9.8.7/tarball"),
        Row::new("npm", "/proxy/reg/pkg"),
        // ── nuget ────────────────────────────────────────────────────────────
        Row::new("nuget", "/proxy/reg/nuget/v3/flat/pkg/index.json"),
        // Axis B here was survey finding 6, and the row that calibrated the axis:
        // the download read `local_svc.storage` directly while the sibling flat
        // *index* checked visibility. It goes through `get_artifact_at_key` now,
        // which gates before it reads.
        Row::new(
            "nuget",
            "/proxy/reg/nuget/v3/flat/pkg/9.8.7/pkg.9.8.7.nupkg",
        ),
        // ── pypi ─────────────────────────────────────────────────────────────
        Row::new("pypi", "/proxy/reg/simple/pkg/").meta(pypi_meta), // was survey finding 9
        // ── conda ────────────────────────────────────────────────────────────
        Row::new("conda", "/proxy/reg/linux-64/pkg-9.8.7-py311_0.conda").meta(conda_meta), // was survey finding 4
        // ── goproxy ──────────────────────────────────────────────────────────
        // Was survey finding 10. `source:read` here, not `releases:read`: a
        // module zip is the source, and the proxy fall-through says so too.
        Row::new("goproxy", "/proxy/reg/example.com/m@v/v9.8.7.zip")
            .coord("example.com/m", "v9.8.7"),
        // Was survey finding 10.
        Row::new("goproxy", "/proxy/reg/example.com/m@v/list")
            .coord("example.com/m", "v9.8.7")
            .meta(go_list_meta),
        // ── rubygems — no finding names it; that is why it is here ───────────
        Row::new("rubygems", "/proxy/reg/gems/pkg-9.8.7.gem"),
        Row::new("rubygems", "/proxy/reg/api/v1/versions/pkg.json"),
        // Was survey finding 16, found by this matrix.
        Row::new("rubygems", "/proxy/reg/info/pkg"),
        // Was survey finding 16. Answers `200` with an *empty* document rather
        // than `403`: it is built by asking each package in turn, and a caller
        // the chain denies is denied every one of them. An empty index discloses
        // nothing, which is what this axis measures.
        Row::new("rubygems", "/proxy/reg/versions").vis(WHOLE_REGISTRY),
        // Same `serve_compact` branch, same empty-document answer. `/names` stays
        // deliberately unfiltered for *blocking* — a gem with one blocked version
        // still exists — which was always a separate question from whether an
        // RBAC-denied caller may read it at all.
        Row::new("rubygems", "/proxy/reg/names").vis(WHOLE_REGISTRY),
        Row::new(
            "rubygems",
            "/proxy/reg/quick/Marshal.4.8/pkg-9.8.7.gemspec.rz",
        )
        .vis(Expect::NotChecked(
            "gem_gemspec has no local branch — it always goes through proxy_stream, so it \
                 never reads the local package whose visibility this axis sets",
        ))
        .no_control(), // needs a stored gemspec, not the flat fixture
        Row::new("rubygems", "/proxy/reg/api/v1/gems/pkg.json"),
        // ── composer ─────────────────────────────────────────────────────────
        Row::new("composer", "/proxy/reg/p2/vendor/pkg.json").pkg("vendor/pkg"),
        Row::new("composer", "/proxy/reg/dist/vendor/pkg/9.8.7").pkg("vendor/pkg"),
        // ── maven ────────────────────────────────────────────────────────────
        Row::new(
            "maven",
            "/proxy/reg/maven2/com/acme/pkg/9.8.7/pkg-9.8.7.jar",
        )
        .pkg("com.acme:pkg")
        .no_control(), // needs a maven multi-file artifact key, not the flat one
        // ── terraform ────────────────────────────────────────────────────────
        // Was survey finding 8.
        Row::new("terraform", "/proxy/reg/v1/providers/acme/vault/versions")
            .pkg("providers/acme/vault")
            .meta(terraform_provider_meta),
        // ── jetbrains marketplace ────────────────────────────────────────────
        // Was survey finding 5.
        Row::new(
            "jetbrains-marketplace",
            "/proxy/reg/pluginManager?action=download&id=org.acme.plugin",
        )
        .pkg("org.acme.plugin")
        .meta(plugin_meta),
        // ── openvsx — the route this class was first found and fixed on ──────
        Row::new("openvsx", "/proxy/reg/acme.ext/9.8.7/vsix")
            .pkg("acme.ext")
            .meta(extension_meta),
        // ── further rows, added after the first pass found rubygems ──────────
        Row::new("npm", "/proxy/reg/-/package/pkg/dist-tags"),
        Row::new("npm", "/proxy/reg/pkg/9.8.7"),
        Row::new("pypi", "/proxy/reg/pypi/pkg/json")
            .meta(pypi_meta)
            .vis(Expect::NotChecked(
                "pypi_json renders from ProxyService::version_document with no local branch, so \
                 it never reads the local package whose visibility this axis sets",
            )),
        Row::new("pypi", "/proxy/reg/packages/pkg-9.8.7.tar.gz").meta(pypi_meta),
        Row::new("nuget", "/proxy/reg/nuget/v3/registration5/pkg/index.json"),
        // Was survey finding 15, found by this matrix: the cargo *sparse index*,
        // the read path of every `cargo build`. `serve_local_index` called
        // `get_index` with no chain while `proxy_upstream_index` directly below
        // it carried a comment recording that this exact gap — "a private cargo
        // registry's crate names and versions were readable by anyone who could
        // reach the port" — was why the proxy path moved onto `ProxyService`.
        // Closed there, left open here.
        Row::new("cargo", "/proxy/reg/registry/pk/g/pkg").meta(cargo_index_meta),
        Row::new("goproxy", "/proxy/reg/example.com/m@latest").coord("example.com/m", "v9.8.7"),
        // Was survey finding 8.
        Row::new(
            "terraform",
            "/proxy/reg/v1/providers/acme/vault/9.8.7/download/linux/amd64",
        )
        .pkg("providers/acme/vault")
        .meta(terraform_provider_meta),
        // ── ecosystems no survey finding names: deb/rpm/pacman, generic, vsx ──
        //
        // The completeness critic's judgement was that these are "far more
        // likely to be *unexamined* than *correct*", which is the whole reason
        // the matrix exists rather than a list of the handlers already known bad.
        Row::new("deb", "/proxy/reg/deb/dists/stable/Release")
            .vis(WHOLE_REGISTRY)
            .no_control(), // repo metadata is generated, not the flat fixture
        Row::new("rpm", "/proxy/reg/rpm/repodata/repomd.xml")
            .vis(WHOLE_REGISTRY)
            .no_control(),
        Row::new("pacman", "/proxy/reg/pacman/reg.db")
            .vis(WHOLE_REGISTRY)
            .no_control(),
        // Axis B is not a finding, and worth stating so nobody re-raises it:
        // `generic` is a path mirror with no local branch at all. Its coordinate
        // is the synthetic `repo/_` with the whole request path as the artifact,
        // so it never reads the local package this axis marks Internal — the
        // `200` is the upstream's file, and the fixture's URL merely contains the
        // string `pkg`.
        Row::new("generic", "/proxy/reg/generic/pkg/9.8.7/file.bin")
            .vis(Expect::NotChecked(
                "path mirror: the coordinate is repo/_ and no local package is read",
            ))
            .no_control(),
        Row::new(
            "vscode-marketplace",
            "/proxy/reg/vscode/asset/acme/ext/9.8.7/Microsoft.VisualStudio.Services.VSIXPackage",
        )
        .pkg("acme.ext")
        .meta(extension_meta)
        .no_control(),
        Row::new(
            "vscode-marketplace",
            "/proxy/reg/vscode/gallery/publishers/acme/vsextensions/ext/9.8.7/vspackage",
        )
        .pkg("acme.ext")
        .meta(extension_meta)
        .no_control(),
        // ── terraform modules (the provider half is covered above) ───────────
        Row::new("terraform", "/proxy/reg/v1/modules/acme/vpc/aws/versions")
            .pkg("modules/acme/vpc/aws")
            .meta(terraform_module_meta)
            .no_control(),
        // ── search / whole-registry indexes ──────────────────────────────────
        // Was survey finding 11. `resolve_and_search` took the identity and
        // dropped it (`let _ = identity`); it now authorises the listing *and*
        // filters the local hits, so both halves of that finding are covered by
        // this row and by `search.rs`'s own tests.
        Row::new("npm", "/proxy/reg/-/v1/search?text=pkg")
            .vis(WHOLE_REGISTRY)
            .no_control(),
        // Was survey finding 11 — the same `resolve_and_search` middle.
        Row::new("nuget", "/proxy/reg/nuget/v3/query?q=pkg")
            .vis(WHOLE_REGISTRY)
            .no_control(),
        Row::new("composer", "/proxy/reg/packages.json")
            .pkg("vendor/pkg")
            .vis(WHOLE_REGISTRY)
            .no_control(),
        // ── jetbrains marketplace: the XML and files routes ──────────────────
        Row::new(
            "jetbrains-marketplace",
            "/proxy/reg/plugins/list?build=IU-241.1",
        )
        .pkg("org.acme.plugin")
        .meta(plugin_meta)
        .vis(WHOLE_REGISTRY)
        .no_control(),
        Row::new("jetbrains-marketplace", "/proxy/reg/updatePlugins.xml")
            .pkg("org.acme.plugin")
            .meta(plugin_meta)
            .vis(WHOLE_REGISTRY)
            .no_control(),
        Row::new(
            "jetbrains-marketplace",
            "/proxy/reg/plugin/download?pluginId=org.acme.plugin&version=9.8.7",
        )
        .pkg("org.acme.plugin")
        .meta(plugin_meta),
    ]
}

/// Build a local-mode registry whose RBAC *allows* anonymous, holding one
/// published package set to `Visibility::Internal`.
///
/// Axis B in isolation: the chain permits the read, so anything that refuses is
/// `check_visibility` and nothing else. `check_visibility` returns `Ok(())`
/// outright when no `team_namespace` port is wired, so the port is the fixture —
/// without it every row would pass vacuously.
async fn vis_app_for(row: &Row) -> impl TestService {
    let ns_store = batlehub_adapters::in_memory::InMemoryTeamNamespaceStore::new();
    let mut parts = local_only_app_parts_with_policy(
        "reg",
        row.kind,
        RegistryMode::Local,
        true,
        rbac_policy_anon_source,
    )
    .await;

    let cur = parts.local_svc.clone();
    parts.local_svc = Arc::new(batlehub_core::services::LocalRegistryService {
        backend: cur.backend.clone(),
        storage: cur.storage.clone(),
        hot: cur.hot.clone(),
        quota: cur.quota.clone(),
        ownership: cur.ownership.clone(),
        team_namespace: Some(ns_store.clone() as Arc<dyn batlehub_core::ports::TeamNamespacePort>),
        sbom: cur.sbom.clone(),
        explore_cache: cur.explore_cache.clone(),
        package_repo: cur.package_repo.clone(),
        readme: cur.readme.clone(),
    });

    seed(&parts, row).await;

    // Internal rather than Team: it refuses anything below `User` with no group
    // setup, so the row tests the gate rather than the fixture's group wiring.
    ns_store
        .set_visibility(
            "reg",
            row.name,
            batlehub_core::entities::Visibility::Internal,
        )
        .await
        .expect("set_visibility");

    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

/// Publish the row's package into `parts`.
async fn seed(parts: &LocalRegistryAppParts, row: &Row) {
    let artifact = Bytes::from_static(b"matrix-fixture-bytes");
    let checksum = hex::encode(Sha256::digest(&artifact));
    parts
        .local_svc
        .publish(PublishRequest {
            registry: "reg".to_owned(),
            name: row.name.to_owned(),
            version: row.version.to_owned(),
            artifact,
            checksum,
            index_metadata: (row.meta)(),
            unlisted: false,
            publisher: Identity {
                user_id: Some("user-1".to_owned()),
                role: Role::User,
                auth_provider: None,
                groups: vec![],
            },
            signature_bytes: None,
            signature_type: None,
        })
        .await
        .expect("matrix fixture must publish");
}

/// Build a local-mode registry denying anonymous everything, holding one
/// published package, and return the app.
async fn app_for(row: &Row) -> impl TestService {
    // `upstream: true` even though every row is a **local**-mode registry.
    // Several local read paths render their document through
    // `ProxyService::version_document`, whose `request_prelude` resolves the
    // registry *client* before authorizing — so with no client the route 400s
    // before the gate is reached and the row asserts nothing. The upstream
    // cannot weaken the negative assertion: the rule chain runs ahead of any
    // fall-through, so an anonymous caller is refused whether or not one exists.
    let parts = local_only_app_parts_with_policy(
        "reg",
        row.kind,
        RegistryMode::Local,
        true,
        rbac_policy_deny_anonymous,
    )
    .await;

    seed(&parts, row).await;
    build_local_registry_app(parts, batlehub_web::CargoIndexMap::default(), None).await
}

// ── Completeness ─────────────────────────────────────────────────────────────
//
// The rows above are only as good as their coverage, and until this section
// existed nothing failed when a route shipped without one. The file's own
// guidance said as much — "a new ecosystem's read routes are not covered until
// someone does" — which is the same *by convention rather than by construction*
// failure the matrix exists to end, moved up one level. A ninth ecosystem would
// have arrived with no rows, a green suite and no signal.
//
// So the inventory below names **every** `GET` this server registers under
// `/proxy/**`, and the test asserts it against the live route table in both
// directions. Adding a route fails until it is classified here; deleting one
// fails until its entry goes. Neither can happen by accident, and neither can
// happen without a reviewer seeing the diff.
//
// # How to extend
//
// 1. Add the route to [`ROUTE_INVENTORY`] — the failure message prints the line.
// 2. Choose its [`Coverage`]:
//    - [`Coverage::Row`] once a row in [`matrix`] makes a real request to it;
//    - [`Coverage::NoPackage`] if the response contains no package, with the
//      reason;
//    - [`Coverage::NoRow`] otherwise — a package read nobody has covered yet.
// 3. If you chose `NoRow`, adding the row is the actual work. `NoRow` is a
//    placeholder that keeps the suite honest, not a resting place.
//
// # Why an exact list rather than pattern-matching rows onto routes
//
// It was written the other way first: derive coverage by testing whether a
// row's concrete URI is routed by a template. That needs a router, and a
// hand-written approximation of one got it wrong in both directions — a path
// parameter is not always a single segment (`{module}` holds
// `example.com/team/lib`), and a greedy matcher then claims coverage that does
// not exist. A wrong "covered" is exactly the outcome this file exists to
// prevent, so the mapping is stated instead of inferred.

/// Whether the matrix exercises a route, and if not, why not.
#[derive(Debug, Clone, Copy)]
enum Coverage {
    /// A row in [`matrix`] makes a real request to this route, so both axes are
    /// asserted against real behaviour.
    Row,
    /// The response contains no package, so neither axis applies: a service
    /// index, a liveness probe, a CVE feed. A claim about the route, so it is
    /// recorded per route rather than inferred from a path prefix.
    NoPackage(&'static str),
    /// A package read with no row yet — the route-level twin of
    /// [`Expect::KnownGap`]. Finite, visible, and meant to shrink.
    NoRow(&'static str),
}

const ROUTE_INVENTORY: &[(&str, Coverage)] = &[
    ("/proxy/{registry}/-/package/{package}/dist-tags", Coverage::Row),
    ("/proxy/{registry}/-/ping", Coverage::NoPackage("npm liveness handshake; answers the same to everyone")),
    ("/proxy/{registry}/-/v1/search", Coverage::Row),
    ("/proxy/{registry}/-/whoami", Coverage::NoPackage("echoes the caller's own identity, never a package")),
    ("/proxy/{registry}/.well-known/terraform.json", Coverage::NoPackage("Terraform service discovery; static endpoint map")),
    ("/proxy/{registry}/api/-/search", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/packages/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/plugins/{id}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/plugins/{id}/updates", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/products/intellij/plugins/{id}/comments", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/search/aggregation/{field}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/search/plugins", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/searchPlugins", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/security-advisories/", Coverage::NoPackage("Composer advisory feed; CVE data, not package contents")),
    ("/proxy/{registry}/api/v1/crates", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/v1/crates/{name}/owners", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/v1/gems/{name}.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/v1/versions/{name}.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/v4/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/api/version", Coverage::NoPackage("JetBrains marketplace API version banner")),
    ("/proxy/{registry}/api/{namespace}", Coverage::Row),
    ("/proxy/{registry}/api/{namespace}/{extension}", Coverage::Row),
    ("/proxy/{registry}/api/{namespace}/{extension}/{version}", Coverage::Row),
    ("/proxy/{registry}/api/{namespace}/{extension}/{version}/file/{filename}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/channeldata.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/deb/{path}", Coverage::Row),
    ("/proxy/{registry}/dist/{vendor}/{package}/{version}", Coverage::Row),
    ("/proxy/{registry}/feature/getImplementations", Coverage::NoPackage("JetBrains feature lookup; no package coordinate in the answer")),
    ("/proxy/{registry}/files/IDE/extensions.json", Coverage::NoPackage("JetBrains IDE-wide bundled-extension manifest")),
    ("/proxy/{registry}/files/brokenPlugins.json", Coverage::NoPackage("JetBrains compatibility blocklist, published upstream for every IDE")),
    ("/proxy/{registry}/files/jbPluginsXMLIds.json", Coverage::NoPackage("JetBrains ID index, upstream-wide")),
    ("/proxy/{registry}/files/pluginsXMLIds.json", Coverage::NoPackage("JetBrains ID index, upstream-wide")),
    ("/proxy/{registry}/files/{plugin}/meta.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/files/{plugin}/{update}/meta.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/files/{plugin}/{update}/{file_name}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/gems/{filename}", Coverage::Row),
    ("/proxy/{registry}/generic/{path}", Coverage::Row),
    ("/proxy/{registry}/info/{gem}", Coverage::Row),
    ("/proxy/{registry}/jetbrains/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/latest_specs.4.8.gz", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/list.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/maven2/{path}", Coverage::Row),
    ("/proxy/{registry}/names", Coverage::Row),
    ("/proxy/{registry}/nuget/v3/autocomplete", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/nuget/v3/flat/{id}/index.json", Coverage::Row),
    ("/proxy/{registry}/nuget/v3/flat/{id}/{version}/{filename}", Coverage::Row),
    ("/proxy/{registry}/nuget/v3/index.json", Coverage::NoPackage("NuGet service index; the list of this registry's own endpoints")),
    ("/proxy/{registry}/nuget/v3/query", Coverage::Row),
    ("/proxy/{registry}/nuget/v3/registration5/{id}/index.json", Coverage::Row),
    ("/proxy/{registry}/nuget/v3/vulnerabilities/index.json", Coverage::NoPackage("NuGet vulnerability feed; CVE data, not package contents")),
    ("/proxy/{registry}/nuget/v3/vulnerabilities/page/{page}", Coverage::NoPackage("NuGet vulnerability feed page; CVE data, not package contents")),
    ("/proxy/{registry}/p2/{path}", Coverage::Row),
    ("/proxy/{registry}/packages.json", Coverage::Row),
    ("/proxy/{registry}/packages/{filename}", Coverage::Row),
    ("/proxy/{registry}/pacman/{path}", Coverage::Row),
    ("/proxy/{registry}/plugin/download", Coverage::Row),
    ("/proxy/{registry}/pluginManager", Coverage::Row),
    ("/proxy/{registry}/plugins/list", Coverage::Row),
    ("/proxy/{registry}/prerelease_specs.4.8.gz", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/pypi/{package}/json", Coverage::Row),
    ("/proxy/{registry}/quick/Marshal.4.8/{filename}", Coverage::Row),
    ("/proxy/{registry}/registry/config.json", Coverage::NoPackage("cargo sparse-index config; names the download endpoint")),
    ("/proxy/{registry}/registry/{path}", Coverage::Row),
    ("/proxy/{registry}/rpm/{path}", Coverage::Row),
    ("/proxy/{registry}/search.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/simple/", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/simple/{package}/", Coverage::Row),
    ("/proxy/{registry}/specs.4.8.gz", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/sumdb/{path}", Coverage::NoPackage("Go checksum-database mirror; upstream notary data, not this registry's packages")),
    ("/proxy/{registry}/updatePlugins.xml", Coverage::Row),
    ("/proxy/{registry}/v1/ID/{id}.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/v1/index.json", Coverage::NoPackage("Terraform service discovery, subpath form")),
    ("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/versions", Coverage::Row),
    ("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}", Coverage::Row),
    ("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/artifact", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/v1/modules/{namespace}/{name}/{provider}/{version}/download", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/v1/providers/{namespace}/{ptype}/versions", Coverage::Row),
    ("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/artifact/{os}/{arch}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/download/{os}/{arch}", Coverage::Row),
    ("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/v1/providers/{namespace}/{ptype}/{version}/shasums.sig", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/versions", Coverage::Row),
    ("/proxy/{registry}/vscode/asset/{publisher}/{name}/{version}/{asset_type}", Coverage::Row),
    ("/proxy/{registry}/vscode/gallery/publishers/{publisher}/vsextensions/{name}/{version}/vspackage", Coverage::Row),
    ("/proxy/{registry}/vscode/item", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/vscode/unpkg/{publisher}/{name}/{version}/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{extension_id}/{version}/vsix", Coverage::Row),
    ("/proxy/{registry}/{hostname}/{namespace}/{ptype}/index.json", Coverage::Row),
    ("/proxy/{registry}/{hostname}/{namespace}/{ptype}/{version}.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{module}/@latest", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{module}/@v/list", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{module}/@v/{filename}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{name}/{version}/download", Coverage::Row),
    ("/proxy/{registry}/{owner}/{repo}/raw/{git_ref}/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/releases", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/releases/assets/{asset_id}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/releases/download/{tag}/{filename}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/releases/tags/{tag}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/tarball/{tag}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{owner}/{repo}/zipball/{tag}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{package}", Coverage::Row),
    ("/proxy/{registry}/{package}/{version}", Coverage::Row),
    ("/proxy/{registry}/{package}/{version}/tarball", Coverage::Row),
    ("/proxy/{registry}/{platform}/current_repodata.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{platform}/repodata.json", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{platform}/repodata.json.bz2", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{platform}/repodata.json.zst", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{platform}/{filename}", Coverage::Row),
    ("/proxy/{registry}/{project}/-/archive/{tag}/{filename}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{project}/-/raw/{git_ref}/{path}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{project}/-/releases", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{project}/-/releases/{tag}", Coverage::NoRow("package read, not yet exercised")),
    ("/proxy/{registry}/{project}/-/releases/{tag}/downloads/{name}", Coverage::NoRow("package read, not yet exercised")),
];

/// The inventory and the router agree, exactly.
///
/// Two failure directions, both of which have to be loud. A route the server
/// registers and the inventory does not name is a read nobody has considered —
/// the whole finding class in one sentence. An entry naming a route the server
/// no longer registers is a claim of coverage over nothing, which hides the day
/// that path comes back under a different handler.
#[test]
fn the_route_inventory_matches_the_router() {
    let spec = batlehub_web::openapi_spec();
    let registered: Vec<String> = spec
        .paths
        .paths
        .iter()
        .filter(|(path, item)| path.starts_with("/proxy/") && item.get.is_some())
        .map(|(path, _)| path.clone())
        .collect();

    let listed: Vec<&str> = ROUTE_INVENTORY.iter().map(|(p, _)| *p).collect();

    let unlisted: Vec<&String> = registered
        .iter()
        .filter(|p| !listed.contains(&p.as_str()))
        .collect();
    let stale: Vec<&str> = listed
        .iter()
        .filter(|p| !registered.iter().any(|r| r == *p))
        .copied()
        .collect();

    let mut report = String::new();
    if !unlisted.is_empty() {
        report.push_str(&format!(
            "\n{} proxy GET route(s) are registered but absent from ROUTE_INVENTORY.\n\
             Every read route has to be classified, or the next ecosystem repeats the\n\
             2026-08-26 survey's finding class in silence. Add:\n\n{}\n\n\
             …then either write a row in `matrix()` and mark it Coverage::Row, or record\n\
             why it needs none.\n",
            unlisted.len(),
            unlisted
                .iter()
                .map(|p| format!(
                    "    (\"{p}\", Coverage::NoRow(\"package read, not yet exercised\")),"
                ))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} ROUTE_INVENTORY entr(ies) name a route this server no longer registers.\n\
             Delete them — a stale entry claims coverage of nothing:\n  {}\n",
            stale.len(),
            stale.join("\n  ")
        ));
    }
    assert!(report.is_empty(), "{report}");
}

/// Every row points at a route that exists, and every `Coverage::Row` has one.
///
/// The two halves can drift apart silently: a row whose URI stops routing
/// anywhere still "passes" its negative assertion, because a `404` reads as
/// *denied*. Its positive control is what normally catches that — this is the
/// cheaper check that says so directly.
#[test]
fn every_row_is_accounted_for_in_the_inventory() {
    let claimed = ROUTE_INVENTORY
        .iter()
        .filter(|(_, c)| matches!(c, Coverage::Row))
        .count();
    let rows = matrix().len();
    assert!(
        claimed > 0 && rows > 0,
        "the matrix and the inventory must both be non-empty"
    );
    assert!(
        rows >= claimed,
        "ROUTE_INVENTORY claims {claimed} routes are exercised by a row, but `matrix()` \
         only has {rows} rows — at least one claim has no test behind it"
    );
}

/// How much of the read surface is actually exercised, printed on every run.
///
/// Not an assertion — a number that goes up. It is here because the honest
/// answer to "is this suite good enough" is a ratio, and the ratio is otherwise
/// invisible: a green suite with 43 of 113 routes covered looks exactly like a
/// green suite with all of them.
#[test]
fn report_read_surface_coverage() {
    let total = ROUTE_INVENTORY.len();
    let rows = ROUTE_INVENTORY
        .iter()
        .filter(|(_, c)| matches!(c, Coverage::Row))
        .count();
    let no_package = ROUTE_INVENTORY
        .iter()
        .filter(|(_, c)| matches!(c, Coverage::NoPackage(_)))
        .count();
    let no_row: Vec<(&str, &str)> = ROUTE_INVENTORY
        .iter()
        .filter_map(|(p, c)| match c {
            Coverage::NoRow(why) => Some((*p, *why)),
            _ => None,
        })
        .collect();
    let denominator = total - no_package;
    println!(
        "authz matrix: {rows}/{denominator} package-read routes exercised \
         ({} with no row, {no_package} disclose no package, {total} total)",
        no_row.len()
    );
    for (path, why) in &no_row {
        println!("  no row: {path} — {why}");
    }
}

/// Every classification states a reason, and every reason says something.
///
/// `NoPackage` and `NoRow` are both assertions about a route that nobody has
/// tested — the only thing standing behind them is the sentence next to them, so
/// an empty one is a classification with no argument at all.
#[test]
fn every_unexercised_route_gives_a_reason() {
    let empty: Vec<&str> = ROUTE_INVENTORY
        .iter()
        .filter_map(|(path, c)| match c {
            Coverage::NoPackage(why) | Coverage::NoRow(why) if why.trim().is_empty() => Some(*path),
            _ => None,
        })
        .collect();
    assert!(
        empty.is_empty(),
        "these routes are excused with no reason given:\n  {}",
        empty.join("\n  ")
    );
}

/// The matrix.
///
/// One test rather than one per row, because the value is in the table being
/// exhaustive and read as a whole; a failure names the row precisely.
#[actix_web::test]
async fn every_local_read_route_enforces_the_registry_rule_chain() {
    let mut failures: Vec<String> = Vec::new();
    let mut broken_controls: Vec<String> = Vec::new();
    let mut gaps_now_fixed: Vec<String> = Vec::new();

    for row in matrix() {
        // ── Axis A: the registry rule chain ──────────────────────────────────
        let app = app_for(&row).await;

        // Positive control: a caller the policy grants must be served, or the
        // negative assertion below proves nothing.
        let mut control_ok = true;
        if row.control {
            let (shown, status) =
                disclosed_as(&app, row.uri, row.name, row.version, Some(USER_TOKEN)).await;
            if !shown {
                broken_controls.push(format!(
                    "[chain] {} {} — a permitted caller did NOT see the package (status {status}). \
                     The fixture or the route is wrong, and the anonymous assertion below proves \
                     nothing until it is fixed.",
                    row.kind, row.uri,
                ));
                control_ok = false;
            }
        }

        if control_ok {
            // Anonymous, whom the registry's RBAC grants nothing.
            let (served, status) = disclosed(&app, row.uri, row.name, row.version).await;
            match row.expect {
                Expect::Denied if served => failures.push(format!(
                    "[chain] {} {} — disclosed the package to a caller the registry's RBAC denies \
                     (status {status})",
                    row.kind, row.uri
                )),
                Expect::KnownGap(_) if !served => gaps_now_fixed.push(format!(
                    "[chain] {} {} — now correctly refuses ({status}). Flip this row to \
                     Expect::Denied.",
                    row.kind, row.uri,
                )),
                _ => {}
            }
        }

        // ── Axis B: per-package visibility ───────────────────────────────────
        if matches!(row.expect_vis, Expect::NotChecked(_)) {
            continue;
        }
        let vapp = vis_app_for(&row).await;

        let mut vis_control_ok = true;
        if row.control {
            // A `User` is above the bar `Visibility::Internal` sets, so the same
            // request must still be served — the control for this axis.
            let (shown, status) =
                disclosed_as(&vapp, row.uri, row.name, row.version, Some(USER_TOKEN)).await;
            if !shown {
                broken_controls.push(format!(
                    "[visibility] {} {} — a permitted caller did NOT see the package (status \
                     {status}); fixture or route is wrong",
                    row.kind, row.uri,
                ));
                vis_control_ok = false;
            }
        }

        if vis_control_ok {
            // Anonymous. The chain allows; only `check_visibility` can refuse.
            let (served, status) = disclosed(&vapp, row.uri, row.name, row.version).await;
            match row.expect_vis {
                Expect::Denied if served => failures.push(format!(
                    "[visibility] {} {} — disclosed an Internal-visibility package to an \
                     anonymous caller (status {status})",
                    row.kind, row.uri
                )),
                Expect::KnownGap(_) if !served => gaps_now_fixed.push(format!(
                    "[visibility] {} {} — now correctly refuses ({status}). Flip this row to \
                     Expect::Denied.",
                    row.kind, row.uri,
                )),
                _ => {}
            }
        }
    }

    let mut report = String::new();
    if !broken_controls.is_empty() {
        report.push_str(&format!(
            "\n{} row(s) whose positive control failed:\n  {}\n",
            broken_controls.len(),
            broken_controls.join("\n  ")
        ));
    }
    if !failures.is_empty() {
        report.push_str(&format!(
            "\n{} NEW authorization gap(s):\n  {}\n",
            failures.len(),
            failures.join("\n  ")
        ));
    }
    if !gaps_now_fixed.is_empty() {
        report.push_str(&format!(
            "\n{} known gap(s) that appear fixed:\n  {}\n",
            gaps_now_fixed.len(),
            gaps_now_fixed.join("\n  ")
        ));
    }
    assert!(report.is_empty(), "{report}");
}
