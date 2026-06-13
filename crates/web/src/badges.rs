//! External badge/link helpers (the `socket_badge` feature flag).

/// Map a registry adapter type to its [socket.dev](https://socket.dev) ecosystem
/// slug, or `None` when socket.dev does not cover that ecosystem.
fn socket_ecosystem(registry_type: &str) -> Option<&'static str> {
    match registry_type {
        "cargo" => Some("cargo"),
        "npm" => Some("npm"),
        "pypi" => Some("pypi"),
        "maven" => Some("maven"),
        "rubygems" => Some("gem"),
        "goproxy" => Some("golang"),
        "nuget" => Some("nuget"),
        "composer" => Some("packagist"),
        _ => None,
    }
}

/// Build the socket.dev badge URL for a package version, e.g.
/// `https://badge.socket.dev/cargo/package/yaml/0.3.0`.
///
/// Returns `None` when the registry type is not covered by socket.dev. The caller
/// is responsible for the per-registry `socket_badge` feature-flag check.
pub fn socket_badge_url(registry_type: &str, name: &str, version: &str) -> Option<String> {
    let eco = socket_ecosystem(registry_type)?;
    Some(format!(
        "https://badge.socket.dev/{eco}/package/{name}/{version}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_url_matches_expected() {
        assert_eq!(
            socket_badge_url("cargo", "yaml", "0.3.0").as_deref(),
            Some("https://badge.socket.dev/cargo/package/yaml/0.3.0")
        );
    }

    #[test]
    fn every_supported_ecosystem_maps() {
        // (registry_type, expected socket.dev ecosystem slug)
        let cases = [
            ("cargo", "cargo"),
            ("npm", "npm"),
            ("pypi", "pypi"),
            ("maven", "maven"),
            ("rubygems", "gem"),
            ("goproxy", "golang"),
            ("nuget", "nuget"),
            ("composer", "packagist"),
        ];
        for (reg_type, eco) in cases {
            assert_eq!(
                socket_badge_url(reg_type, "pkg", "1.0.0").as_deref(),
                Some(format!("https://badge.socket.dev/{eco}/package/pkg/1.0.0").as_str()),
                "registry type {reg_type} should map to ecosystem {eco}",
            );
        }
    }

    #[test]
    fn unsupported_types_are_none() {
        for reg_type in [
            "github",
            "terraform",
            "openvsx",
            "conda",
            "vscode-marketplace",
            "unknown",
        ] {
            assert!(
                socket_badge_url(reg_type, "mod", "1.0.0").is_none(),
                "registry type {reg_type} is not on socket.dev and must be None",
            );
        }
    }
}
