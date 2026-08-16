/**
 * How a registry type addresses its artifacts.
 *
 * Most registries are addressed by *package and version* (`lodash@4.17.21`).
 * A handful pass the upstream path straight through and have no per-package
 * version model at all (`idea/idea-2026.1.3.tar.gz`) — `deb`, `rpm`, `pacman`,
 * `jetbrains` and `generic`, which is also the set the backend's `path_allow`
 * allowlist exists for.
 *
 * RFC 0004-bis §6.1: `AdminWarming` rendered eleven identical cards, each with
 * both a package field and a path field, and a JetBrains path placeholder on
 * cargo and npm — a placeholder suggesting an input the registry cannot accept.
 * That is PRODUCT principle 5 ("registry types are data") going unenforced.
 * The difference is a property of the type, so it lives here as data rather
 * than as a condition written out per page.
 */

/** Registry types whose artifacts are named by an upstream path. */
export const PATH_ADDRESSED_TYPES = new Set(["deb", "rpm", "pacman", "jetbrains", "generic"]);

export type Addressing = "package" | "path";

export function addressingOf(registryType: string | undefined): Addressing {
  return registryType && PATH_ADDRESSED_TYPES.has(registryType) ? "path" : "package";
}
