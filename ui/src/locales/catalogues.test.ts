import { describe, expect, it } from "vitest";

import en from "./en.json";
import fr from "./fr.json";
import {
  ADMIN_SIDEBAR,
  NAMESPACES_TABS,
  OBSERVABILITY_TABS,
  OPERATIONS_TABS,
  PACKAGES_TABS,
  SECURITY_TABS,
} from "@/config/adminSections";
import { TOOLS_TABS, accountTabs, primaryNav } from "@/config/navigation";
import { VISIBILITY_OPTIONS } from "@/config/visibility";

/**
 * The catalogue gate (RFC 0003 §4.6, §10).
 *
 * A missing key does not crash — vue-i18n falls back to English — which is
 * precisely why this has to be a test. Without it, a French user quietly reads
 * half an English interface and nothing ever fails.
 */

type Tree = Record<string, unknown>;

function flatten(tree: Tree, prefix = ""): string[] {
  return Object.entries(tree).flatMap(([key, value]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return value !== null && typeof value === "object" ? flatten(value as Tree, path) : [path];
  });
}

function valueAt(tree: Tree, path: string): unknown {
  return path.split(".").reduce<unknown>((node, key) => (node as Tree)?.[key], tree);
}

const enKeys = flatten(en as Tree).sort();
const frKeys = flatten(fr as Tree).sort();

/** `{name}`-style placeholders, which must survive translation intact. */
const placeholders = (text: string): string[] =>
  [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("locale catalogues", () => {
  it("have identical key sets", () => {
    expect(frKeys).toEqual(enKeys);
  });

  it("have no empty strings", () => {
    for (const locale of [
      ["en", en],
      ["fr", fr],
    ] as const) {
      for (const key of enKeys) {
        const value = valueAt(locale[1] as Tree, key);
        expect(typeof value, `${locale[0]}.${key}`).toBe("string");
        expect((value as string).trim().length, `${locale[0]}.${key} is empty`).toBeGreaterThan(0);
      }
    }
  });

  /**
   * A dropped placeholder is the failure mode a human translator hits most, and
   * it renders as a sentence with a hole in it rather than as an error.
   */
  it("keep every interpolation placeholder in the translation", () => {
    for (const key of enKeys) {
      const source = valueAt(en as Tree, key) as string;
      const target = valueAt(fr as Tree, key) as string;
      expect(placeholders(target), `${key}: placeholders must match the English source`).toEqual(
        placeholders(source),
      );
    }
  });

  /**
   * Domain terms stay verbatim in both catalogues (RFC 0003 §4.6). A French UI
   * that renames `config.toml` or translates a registry mode leaves the reader
   * unable to search for it, type it, or match it against the docs — and a
   * mistranslated destructive verb is a safety problem, not a cosmetic one.
   */
  it("never translates a domain term", () => {
    const VERBATIM = ["config.toml", "[[registries]]", "ConfigMap", "Helm", "TOML", "CLI"];
    for (const key of enKeys) {
      const source = valueAt(en as Tree, key) as string;
      const target = valueAt(fr as Tree, key) as string;
      for (const term of VERBATIM) {
        if (source.includes(term)) {
          expect(target, `${key} drops the domain term "${term}"`).toContain(term);
        }
      }
    }
  });

  /**
   * Navigation labels are catalogue *keys*, resolved with `t()` at render.
   * They used to be English literals, and because the i18n audit only read
   * templates it could not see them: the entire admin navigation rendered in
   * English while the gate reported zero untranslated strings. A key that stops
   * resolving renders as the literal text `adminNav.users`, so this is the test
   * that turns that back into a build failure.
   */
  it("resolves every navigation label to a real key in both catalogues", () => {
    const viewers = [
      { isAuthenticated: true, isAdmin: true, hasRegistryAccess: true },
      { isAuthenticated: false, isAdmin: false, hasRegistryAccess: true },
    ];
    const labels = [
      ...ADMIN_SIDEBAR,
      ...PACKAGES_TABS,
      ...SECURITY_TABS,
      ...NAMESPACES_TABS,
      ...OPERATIONS_TABS,
      ...OBSERVABILITY_TABS,
      ...TOOLS_TABS,
      ...VISIBILITY_OPTIONS,
      ...accountTabs({ isOidc: true }),
      ...viewers.flatMap((v) => primaryNav(v)),
    ].map((entry) => entry.label);

    expect(labels.length).toBeGreaterThan(20);
    for (const label of labels) {
      expect(label, `"${label}" is a literal, not a catalogue key`).toMatch(/^[a-z]\w*(\.\w+)+$/);
      expect(enKeys, `en is missing ${label}`).toContain(label);
      expect(frKeys, `fr is missing ${label}`).toContain(label);
    }
  });

  /**
   * `@` opens a linked-message reference in vue-i18n (`@:other.key`), so a
   * literal one has to go through literal interpolation — `{'@'}`. Unescaped,
   * the message fails to *compile*, and vue-i18n takes the surrounding messages
   * down with it: `react@18.0.0` in a placeholder threw "Invalid linked format"
   * and broke the admin warming page at runtime. Nothing type-checks this, and
   * no test rendered that page, so it only surfaced under an authenticated
   * accessibility scan.
   */
  it("escapes any literal @, which vue-i18n would read as a linked message", () => {
    const offenders = enKeys.filter((key) =>
      [en, fr].some((cat) => {
        const value = valueAt(cat as Tree, key) as string;
        // `{'@'}` is the escaped form; anything else containing @ is a bug.
        return /@/.test(value.replace(/\{'@'\}/g, ""));
      }),
    );
    expect(offenders, "use {'@'} for a literal @").toEqual([]);
  });

  /** French runs longer than English; this is the budget the layouts assume. */
  it("keeps French within 60% of the English length", () => {
    const offenders = enKeys.filter((key) => {
      const source = (valueAt(en as Tree, key) as string).length;
      const target = (valueAt(fr as Tree, key) as string).length;
      return source >= 20 && target > source * 1.6;
    });
    expect(offenders, "these will overflow layouts sized for English").toEqual([]);
  });
});
