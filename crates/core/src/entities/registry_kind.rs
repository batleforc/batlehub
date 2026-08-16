use serde::{Deserialize, Serialize};

/// Whether blocked versions are removed from one listing document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingSupport {
    /// Blocked versions are absent from this document, and whatever it calls
    /// "newest" is repaired to name a version that is still allowed.
    Filtered,
    /// Filtered, with a caveat an operator has to know about. The string
    /// completes "yes — …".
    Qualified(&'static str),
    /// Not filtered, and why. The string completes "no — …".
    ///
    /// The reason travels with the code that decides it so the published
    /// coverage table cannot drift from the behaviour. Two reasons exist today:
    /// editing a signed repository index invalidates its signature and the
    /// client rejects the whole repository (a worse failure than the one
    /// filtering fixes), and RubyGems' Marshal indexes would need a Ruby
    /// Marshal encoder in Rust to hide a version its JSON APIs already hide for
    /// every client released this decade.
    Unsupported(&'static str),
}

/// One version-listing document a registry kind serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListingDocument {
    /// How the admin guide names it — "packument", "`maven-metadata.xml`".
    pub label: &'static str,
    pub support: ListingSupport,
}

impl ListingDocument {
    const fn filtered(label: &'static str) -> Self {
        Self {
            label,
            support: ListingSupport::Filtered,
        }
    }
    const fn qualified(label: &'static str, note: &'static str) -> Self {
        Self {
            label,
            support: ListingSupport::Qualified(note),
        }
    }
    const fn unsupported(label: &'static str, reason: &'static str) -> Self {
        Self {
            label,
            support: ListingSupport::Unsupported(reason),
        }
    }
}

/// The protocol a registry adapter speaks — e.g. `"cargo"`, `"npm"`, `"maven"`.
///
/// Distinct from a registry's user-configured *instance* name (e.g. `"my-maven"`
/// in `RegistryConfig.name`, or `RegistryMap`'s keys): many instances of the same
/// type can be configured, each proxying a different upstream.
///
/// Serializes/deserializes as the same kebab-case strings the TOML config and
/// wire format already use, so this is a drop-in replacement for the bare
/// `String` — the one place those strings must stay in sync (this enum) is now
/// compiler-enforced instead of hand-synced across `crates/config`'s validator
/// and `server/src/builders.rs`'s client-construction match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryKind {
    Github,
    Forgejo,
    Gitlab,
    Cargo,
    Npm,
    Openvsx,
    Goproxy,
    Pypi,
    Conda,
    Composer,
    VscodeMarketplace,
    Maven,
    Terraform,
    Rubygems,
    Nuget,
    Deb,
    Rpm,
    Pacman,
    Jetbrains,
    JetbrainsMarketplace,
    Generic,
}

impl RegistryKind {
    /// All known registry kinds, in the same order the config validator and
    /// `server/src/builders.rs` have historically listed them.
    pub const ALL: &'static [RegistryKind] = &[
        Self::Github,
        Self::Forgejo,
        Self::Gitlab,
        Self::Cargo,
        Self::Npm,
        Self::Openvsx,
        Self::Goproxy,
        Self::Pypi,
        Self::Conda,
        Self::Composer,
        Self::VscodeMarketplace,
        Self::Maven,
        Self::Terraform,
        Self::Rubygems,
        Self::Nuget,
        Self::Deb,
        Self::Rpm,
        Self::Pacman,
        Self::Jetbrains,
        Self::JetbrainsMarketplace,
        Self::Generic,
    ];

    /// The kebab-case wire string for this kind (matches TOML `type = "..."`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Forgejo => "forgejo",
            Self::Gitlab => "gitlab",
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Openvsx => "openvsx",
            Self::Goproxy => "goproxy",
            Self::Pypi => "pypi",
            Self::Conda => "conda",
            Self::Composer => "composer",
            Self::VscodeMarketplace => "vscode-marketplace",
            Self::Maven => "maven",
            Self::Terraform => "terraform",
            Self::Rubygems => "rubygems",
            Self::Nuget => "nuget",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Pacman => "pacman",
            Self::Jetbrains => "jetbrains",
            Self::JetbrainsMarketplace => "jetbrains-marketplace",
            Self::Generic => "generic",
        }
    }

    /// Local/hybrid mode is only meaningful for registries this proxy can host
    /// package versions for itself — the read-only source-hosting types
    /// (github/forgejo/gitlab/jetbrains) have no local publish model. `generic`
    /// is proxy-only for now; hosting arbitrary files is a separate roadmap item.
    pub fn supports_local_mode(&self) -> bool {
        !matches!(
            self,
            Self::Github | Self::Forgejo | Self::Gitlab | Self::Jetbrains | Self::Generic
        )
    }

    /// `deb`/`rpm` have no universal default upstream (unlike e.g. `npm`'s
    /// registry.npmjs.org), so proxy mode requires an explicit upstream —
    /// otherwise every fetch would silently hit an unreachable placeholder.
    /// `generic` mirrors an arbitrary file tree, so it has no default at all.
    pub fn requires_explicit_upstream_in_proxy_mode(&self) -> bool {
        matches!(self, Self::Deb | Self::Rpm | Self::Generic)
    }

    /// Whether this kind is addressed purely by upstream file path, with the
    /// whole path carried in `PackageId::artifact` and no per-package metadata
    /// API — the kinds served by `PathProxyRegistryClient`. These are the kinds
    /// for which `path_allow` and `cache.warm_paths` are meaningful.
    pub fn is_path_addressed(&self) -> bool {
        matches!(
            self,
            Self::Deb | Self::Rpm | Self::Pacman | Self::Jetbrains | Self::Generic
        )
    }

    /// The version-listing documents this kind serves, and whether blocked
    /// versions are hidden from each.
    ///
    /// The single source of truth for "which registries filter their listings".
    /// The coverage table in `docs/guide/admin-policies.md` is *generated* from
    /// this rather than maintained beside it, because the previous arrangement —
    /// a warning box in the admin guide naming npm as the only filtered
    /// ecosystem — was a fact about the code that nothing kept true.
    ///
    /// Exhaustive with no wildcard arm, so adding a registry kind does not
    /// compile until it answers the question, in the same way
    /// `server/src/builders.rs`'s match already forces a decision about client
    /// construction. An empty slice is a legitimate answer and means the
    /// protocol has no version listing at all.
    ///
    /// `blocking::strip` is checked against this in
    /// `every_advertised_filter_is_reachable_from_dispatch`: a document
    /// advertised here as filtered must reach a real filter.
    pub fn listing_filter(&self) -> &'static [ListingDocument] {
        // Named consts rather than inline slice literals: a `&[...]` holding
        // const-fn calls is not promoted to `'static`, so each arm needs a
        // const item to borrow from.
        const NPM: &[ListingDocument] = &[ListingDocument::filtered("packument")];
        const NUGET: &[ListingDocument] = &[
            ListingDocument::filtered("flat index"),
            ListingDocument::qualified(
                "registration pages",
                "inline pages only; paged registrations pass through, and are logged",
            ),
        ];
        const MAVEN: &[ListingDocument] = &[ListingDocument::filtered("`maven-metadata.xml`")];
        const PYPI: &[ListingDocument] = &[ListingDocument::filtered(
            "simple index (HTML and PEP 691 JSON)",
        )];
        const CARGO: &[ListingDocument] = &[ListingDocument::qualified(
            "sparse index",
            "blocked versions are marked `yanked` rather than removed, which is cargo's own \
             mechanism for \"exists, do not select\" and keeps lockfile diagnostics honest",
        )];
        const GOPROXY: &[ListingDocument] = &[ListingDocument::filtered("`@v/list` and `@latest`")];
        const RUBYGEMS: &[ListingDocument] = &[
            ListingDocument::filtered("versions and gem JSON APIs"),
            ListingDocument::unsupported(
                "`specs.4.8.gz`, `quick/Marshal.4.8`",
                "hiding a version from a Ruby Marshal index would need a Marshal encoder in \
                 Rust, to hide what the JSON APIs already hide for every client released this \
                 decade",
            ),
        ];
        const COMPOSER: &[ListingDocument] = &[ListingDocument::filtered("p2 metadata")];
        const TERRAFORM: &[ListingDocument] =
            &[ListingDocument::filtered("module and provider versions")];
        const CONDA: &[ListingDocument] = &[ListingDocument::filtered(
            "`repodata.json`, `current_repodata.json`",
        )];
        const JETBRAINS_MARKETPLACE: &[ListingDocument] = &[ListingDocument::filtered(
            "`updatePlugins.xml`, `/plugins/list` and the plugin-updates API",
        )];
        const FORGE: &[ListingDocument] = &[ListingDocument::filtered("release listings")];
        const SIGNED: &[ListingDocument] = &[ListingDocument::unsupported(
            "signed repository indexes",
            "editing one invalidates its signature and the client rejects the whole \
             repository, which is a worse failure than the one filtering fixes",
        )];
        const EXTENSION_GALLERY: &[ListingDocument] = &[ListingDocument::filtered(
            "extension gallery (`extensionquery`) and the OpenVSX API",
        )];

        match self {
            Self::Npm => NPM,
            Self::Nuget => NUGET,
            Self::Maven => MAVEN,
            Self::Pypi => PYPI,
            Self::Cargo => CARGO,
            Self::Goproxy => GOPROXY,
            Self::Rubygems => RUBYGEMS,
            Self::Composer => COMPOSER,
            Self::Terraform => TERRAFORM,
            Self::Conda => CONDA,
            Self::JetbrainsMarketplace => JETBRAINS_MARKETPLACE,
            Self::Github | Self::Gitlab | Self::Forgejo => FORGE,
            Self::Deb | Self::Rpm | Self::Pacman => SIGNED,
            Self::Openvsx | Self::VscodeMarketplace => EXTENSION_GALLERY,
            // `generic` and `jetbrains` mirror an arbitrary file tree by path —
            // there is no listing document in the protocol at all, so there is
            // nothing to say beyond that. (JetBrains *plugins* are the separate
            // `jetbrains-marketplace` kind above, and are filtered; VS Code
            // extensions are `openvsx`/`vscode-marketplace` above.)
            Self::Generic | Self::Jetbrains => &[],
        }
    }

    /// The `PackageId::artifact` sub-coordinate this kind's primary downloadable
    /// artifact is cached under, when it uses one.
    ///
    /// Proxy cache keys are `artifact:` + [`crate::entities::PackageId::cache_key`], i.e.
    /// `artifact:{registry}/{name}/{version}[/{artifact}]`. Most kinds address
    /// their main artifact by name/version alone and return `None` (new kinds
    /// default to that). JetBrains Marketplace serves the plugin archive under
    /// `plugin` — see `jbm_plugin_download` in
    /// `crates/web/src/handlers/proxy/jetbrains_marketplace/files.rs`, which
    /// must keep using the same sub-coordinate.
    ///
    /// Warming reads this so a pre-fetched artifact lands in the exact slot the
    /// proxy read path looks in, instead of a slot nothing ever reads.
    pub fn warm_artifact(&self) -> Option<&'static str> {
        match self {
            Self::JetbrainsMarketplace => Some("plugin"),
            // Both extension kinds read with `.with_artifact("vsix")` in
            // `handlers/proxy/openvsx.rs`. Returning `None` here — as this did —
            // made the warmer write `artifact:{registry}/{name}/{version}` while
            // every read looked in `…/{version}/vsix`, so warming an extension
            // filled a slot nothing ever read and the first real request still
            // went upstream.
            Self::Openvsx | Self::VscodeMarketplace => Some("vsix"),
            _ => None,
        }
    }
}

impl std::str::FromStr for RegistryKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .find(|k| k.as_str() == s)
            .copied()
            .ok_or_else(|| format!("unknown registry type: '{s}'"))
    }
}

impl std::fmt::Display for RegistryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_round_trips_every_variant() {
        for kind in RegistryKind::ALL {
            let s = kind.as_str();
            assert_eq!(s.parse::<RegistryKind>().unwrap(), *kind);
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert!("not-a-real-type".parse::<RegistryKind>().is_err());
    }

    #[test]
    fn serde_round_trips_kebab_case() {
        for kind in RegistryKind::ALL {
            let json = serde_json::to_string(kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
            let back: RegistryKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, *kind);
        }
    }

    #[test]
    fn local_mode_support_matches_source_hosting_exclusion() {
        assert!(!RegistryKind::Github.supports_local_mode());
        assert!(!RegistryKind::Forgejo.supports_local_mode());
        assert!(!RegistryKind::Gitlab.supports_local_mode());
        assert!(!RegistryKind::Jetbrains.supports_local_mode());
        assert!(!RegistryKind::Generic.supports_local_mode());
        assert!(RegistryKind::Cargo.supports_local_mode());
        assert!(RegistryKind::Deb.supports_local_mode());
        assert!(RegistryKind::JetbrainsMarketplace.supports_local_mode());
    }

    #[test]
    fn only_deb_rpm_and_generic_require_explicit_upstream_in_proxy_mode() {
        assert!(RegistryKind::Deb.requires_explicit_upstream_in_proxy_mode());
        assert!(RegistryKind::Rpm.requires_explicit_upstream_in_proxy_mode());
        assert!(RegistryKind::Generic.requires_explicit_upstream_in_proxy_mode());
        assert!(!RegistryKind::Pacman.requires_explicit_upstream_in_proxy_mode());
        assert!(!RegistryKind::Npm.requires_explicit_upstream_in_proxy_mode());
        assert!(!RegistryKind::JetbrainsMarketplace.requires_explicit_upstream_in_proxy_mode());
    }

    #[test]
    fn every_kind_answers_the_listing_filter_question() {
        for kind in RegistryKind::ALL {
            let docs = kind.listing_filter();
            if matches!(kind, RegistryKind::Generic | RegistryKind::Jetbrains) {
                assert!(
                    docs.is_empty(),
                    "{kind} is path-addressed and has no listing document"
                );
                continue;
            }
            assert!(
                !docs.is_empty(),
                "{kind} names no listing document; if it genuinely has none, say so here"
            );
            for d in docs {
                assert!(!d.label.is_empty(), "{kind}: a document needs a name");
                if let ListingSupport::Unsupported(reason) | ListingSupport::Qualified(reason) =
                    d.support
                {
                    assert!(
                        !reason.is_empty(),
                        "{kind}/{}: the published table prints this reason verbatim",
                        d.label
                    );
                }
            }
        }
    }

    /// Every "no" row of the coverage table, and only those. Each one is a
    /// deliberate decision with a reason an operator can read, not an omission.
    #[test]
    fn the_unfiltered_documents_are_the_ones_we_decided_not_to_filter() {
        let unfiltered: Vec<&str> = RegistryKind::ALL
            .iter()
            .filter(|k| {
                k.listing_filter()
                    .iter()
                    .any(|d| matches!(d.support, ListingSupport::Unsupported(_)))
            })
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            unfiltered,
            [
                // Marshal indexes only; the JSON APIs are filtered.
                "rubygems", // Signed repository indexes.
                "deb", "rpm", "pacman",
            ]
        );
    }

    #[test]
    fn path_addressed_kinds_are_the_path_proxy_ones() {
        for kind in [
            RegistryKind::Deb,
            RegistryKind::Rpm,
            RegistryKind::Pacman,
            RegistryKind::Jetbrains,
            RegistryKind::Generic,
        ] {
            assert!(kind.is_path_addressed(), "{kind} should be path-addressed");
        }
        for kind in [
            RegistryKind::Npm,
            RegistryKind::Cargo,
            RegistryKind::Github,
            RegistryKind::Goproxy,
            RegistryKind::JetbrainsMarketplace,
        ] {
            assert!(
                !kind.is_path_addressed(),
                "{kind} should not be path-addressed"
            );
        }
    }
}
