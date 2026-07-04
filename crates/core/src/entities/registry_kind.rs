use serde::{Deserialize, Serialize};

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
        }
    }

    /// Local/hybrid mode is only meaningful for registries this proxy can host
    /// package versions for itself — the read-only source-hosting types
    /// (github/forgejo/gitlab/jetbrains) have no local publish model.
    pub fn supports_local_mode(&self) -> bool {
        !matches!(
            self,
            Self::Github | Self::Forgejo | Self::Gitlab | Self::Jetbrains
        )
    }

    /// `deb`/`rpm` have no universal default upstream (unlike e.g. `npm`'s
    /// registry.npmjs.org), so proxy mode requires an explicit upstream —
    /// otherwise every fetch would silently hit an unreachable placeholder.
    pub fn requires_explicit_upstream_in_proxy_mode(&self) -> bool {
        matches!(self, Self::Deb | Self::Rpm)
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
        assert!(RegistryKind::Cargo.supports_local_mode());
        assert!(RegistryKind::Deb.supports_local_mode());
    }

    #[test]
    fn only_deb_and_rpm_require_explicit_upstream_in_proxy_mode() {
        assert!(RegistryKind::Deb.requires_explicit_upstream_in_proxy_mode());
        assert!(RegistryKind::Rpm.requires_explicit_upstream_in_proxy_mode());
        assert!(!RegistryKind::Pacman.requires_explicit_upstream_in_proxy_mode());
        assert!(!RegistryKind::Npm.requires_explicit_upstream_in_proxy_mode());
    }
}
