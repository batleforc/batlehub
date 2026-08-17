import { describe, expect, it } from "vitest";
import { REGISTRY_TYPE_DEFS, hostOf, netrcHostsFor, type SnippetContext } from "./registryTypes";

/**
 * Host-based routing (RFC 0001) puts each registry on its own subdomain and, on
 * such a host, prefixes `/proxy/{registry}` to *every* path itself. Two things
 * follow for the setup snippets, and both are silent failures when broken:
 *
 * 1. A snippet must never write a literal `/proxy/…` — on a registry host that
 *    becomes `/proxy/npm1/proxy/npm1/…` and 404s.
 * 2. A `.netrc` stanza is matched by hostname. A snippet that points a tool at
 *    several registries reaches several hosts, so it needs a stanza for each;
 *    one stanza would leave the rest anonymous and every authenticated install
 *    would 401 on the first package fetched from another host.
 */

const BASE_HOST = "batlehub.example.com";
const BASE = `https://${BASE_HOST}`;

/** A fully host-routed deployment: every registry lives on its own subdomain. */
const urlFor = (name: string) => `https://${name}.${BASE_HOST}`;

function contextFor(registryName: string): SnippetContext {
  const registryUrl = urlFor(registryName);
  return {
    base: BASE,
    registryName,
    registryUrl,
    urlFor,
    mode: "hybrid",
    isAuthenticated: true,
    token: "tok-abc",
    netrcHost: hostOf(registryUrl),
    netrcLogin: "alice",
    identity: null,
    // The composite `mise` tab renders rules for three registries at once.
    selectedNames: { github: "gh1", npm: "npm1", cargo: "cargo1" },
  };
}

/** Hosts belonging to this deployment that the snippet actually sends requests to. */
function batlehubHosts(rendered: string): string[] {
  const urls = rendered.match(/https?:\/\/[^\s"'`,)]+/g) ?? [];
  const hosts = urls
    .map(hostOf)
    // Upstream hosts (api.github.com, open-vsx.org, …) appear as the *match*
    // side of a rewrite rule; only the proxy side is ours to authenticate.
    .filter((h) => h === BASE_HOST || h.endsWith(`.${BASE_HOST}`));
  return [...new Set(hosts)];
}

/** Hosts the snippet tells the reader to put in `~/.netrc` (or apt's auth.conf). */
function declaredMachines(rendered: string): string[] {
  const machines = rendered.match(/^#?\s*machine\s+(\S+)/gm) ?? [];
  return [...new Set(machines.map((m) => m.replace(/^#?\s*machine\s+/, "")))];
}

describe.each(REGISTRY_TYPE_DEFS)("$id snippets under host-based routing", (def) => {
  const ctx = contextFor(`${def.id.replace(/[^a-z0-9-]/g, "-")}1`);
  const snippets = def.snippets.filter((s) => !s.showWhen || s.showWhen(ctx));

  it("never writes a literal /proxy/ path", () => {
    for (const snippet of snippets) {
      expect(snippet.template(ctx), `${def.id} · ${snippet.key}`).not.toContain("/proxy/");
    }
  });

  it("declares a .netrc stanza for every host it points a tool at", () => {
    for (const snippet of snippets) {
      const rendered = snippet.template(ctx);
      const machines = declaredMachines(rendered);
      // A snippet is free to say nothing about credentials; one that does must
      // say it about all of them.
      if (machines.length === 0) continue;
      for (const host of batlehubHosts(rendered)) {
        expect(machines, `${def.id} · ${snippet.key} — no stanza for ${host}`).toContain(host);
      }
    }
  });
});

describe("netrcHostsFor", () => {
  it("lists the main host first, then each host-routed registry", () => {
    expect(
      netrcHostsFor(BASE, [
        { public_url: `https://npm1.${BASE_HOST}` },
        { public_url: null },
        { public_url: `https://cargo1.${BASE_HOST}` },
      ]),
    ).toEqual([BASE_HOST, `npm1.${BASE_HOST}`, `cargo1.${BASE_HOST}`]);
  });

  it("keeps a vanity host that shares no domain with the server", () => {
    expect(netrcHostsFor(BASE, [{ public_url: "https://npm.acme.io" }])).toEqual([
      BASE_HOST,
      "npm.acme.io",
    ]);
  });

  it("emits one entry per host when several registries share one", () => {
    expect(
      netrcHostsFor(BASE, [
        { public_url: `https://npm1.${BASE_HOST}` },
        { public_url: `https://npm1.${BASE_HOST}` },
      ]),
    ).toEqual([BASE_HOST, `npm1.${BASE_HOST}`]);
  });

  it("is just the main host when nothing is host-routed", () => {
    expect(netrcHostsFor(BASE, [{ public_url: null }, {}])).toEqual([BASE_HOST]);
  });
});
