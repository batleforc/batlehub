import { describe, expect, it } from "vitest";

import { REGISTRY_TYPE_DEFS, type SnippetContext } from "./registryTypes";

/**
 * RFC 0012 O6. Terraform fetches the provider archive with no `Authorization`
 * header and cannot send one, so this page used to tell every Terraform
 * operator to grant anonymous reads across the whole registry.
 *
 * When the registry signs its download URLs that advice is not merely
 * unnecessary, it is wrong — and "open this up" is the kind of wrong that gets
 * followed. These pin that the note reads the flag.
 */
function ctx(signedDownloads: boolean): SnippetContext {
  return {
    base: "https://batlehub.example.com",
    registryName: "tf",
    registryUrl: "https://batlehub.example.com/proxy/tf",
    urlFor: (name) => `https://batlehub.example.com/proxy/${name}`,
    mode: "proxy",
    isAuthenticated: true,
    token: "t",
    netrcHost: "batlehub.example.com",
    netrcLogin: "u",
    identity: null,
    selectedNames: {},
    signedDownloads,
  };
}

function terraformMirrorNote(signed: boolean): string {
  const def = REGISTRY_TYPE_DEFS.find((d) => d.id === "terraform");
  expect(def, "terraform registry type must exist").toBeTruthy();
  const snippet = def!.snippets.find((s) => s.key === "terraformrc");
  expect(snippet, "the ~/.terraformrc snippet must exist").toBeTruthy();
  const note = snippet!.note;
  return typeof note === "function" ? note(ctx(signed)) : (note ?? "");
}

describe("terraform mirror note", () => {
  it("tells an unsigned registry it needs the anonymous grant", () => {
    const note = terraformMirrorNote(false);
    expect(note).toContain("anonymous");
    expect(note).toContain("releases:read");
    // …and offers the alternative rather than presenting it as the only way.
    expect(note).toContain("signed_downloads");
  });

  it("does not ask a signing registry to open itself", () => {
    const note = terraformMirrorNote(true);
    // It may *mention* the grant — "no anonymous grant is needed" is worth
    // saying to someone who read the old advice. What it must not do is hand
    // over the config to paste.
    expect(note).not.toContain("releases:read");
    expect(note).not.toContain("needs <code>anonymous");
    expect(note).toContain("signs its download URLs");
    expect(note).toContain("stay closed");
  });

  it("says something either way", () => {
    for (const signed of [true, false]) {
      expect(terraformMirrorNote(signed).length).toBeGreaterThan(80);
    }
  });
});
