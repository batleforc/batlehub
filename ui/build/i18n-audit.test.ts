import { describe, it, expect } from "vitest";

// @ts-expect-error — plain ESM build script, no type declarations by design.
import { scanSource } from "./i18n-audit.mjs";

const scan = (source: string, isVue = true): string[] => scanSource(source, isVue) as string[];

const vue = (script: string, template = "") =>
  `<script setup lang="ts">\n${script}\n</script>\n\n<template>\n${template}\n</template>\n`;

/**
 * The gate's own regression signal.
 *
 * `pnpm run i18n:check` reported `0 untranslated strings across 0 files` while
 * five English strings rendered to a French operator, and it had reported the
 * same zero once before while the *entire admin navigation* was in English. Both
 * times the scanner was correct about everything it looked at and silent about
 * where the text actually was.
 *
 * So the assertions here are about what the scanner can *see*, not about the
 * current count. A count of zero over a tree proves nothing unless something
 * independently proves a non-zero is reachable — which is exactly the property
 * RFC 0004-bis §2 says four of our gates were missing.
 */
describe("i18n audit", () => {
  describe("finds text wherever it is written", () => {
    it("a component prop carrying English fails", () => {
      // §2.1 verbatim: this rendered a "Clear Cache" button above a French
      // confirmation sentence, and the gate read zero — `confirm-label` was not
      // one of the six attribute names it knew.
      const findings = scan(vue("", `<ConfirmDialog confirm-label="Clear Cache" />`));
      expect(findings).toContain('confirm-label="Clear Cache"');
    });

    it("an attribute name nobody enumerated still fails", () => {
      // The rule, not the list: `looksHuman` accepts any `*-label`/`*-message`
      // name, so a prop invented tomorrow is covered without a scanner change.
      const findings = scan(vue("", `<Thing rollback-message="Nothing to roll back." />`));
      expect(findings).toContain('rollback-message="Nothing to roll back."');
    });

    it("a string assigned to a ref fails", () => {
      const findings = scan(vue(`error.value = "Please enter a token.";`));
      expect(findings).toContain('error.value = "Please enter a token."');
    });

    it("a string in an object-literal lookup table fails", () => {
      // `theme.dark` was translated, correct and referenced zero times while
      // `ThemeToggle` held this map. Scanning `label:`/`title:` only is what
      // made the key's *name* decide whether its value counted as text.
      const findings = scan(vue(`const LABELS = { dark: "Theme: dark" };`));
      expect(findings).toContain('dark: "Theme: dark"');
    });

    it("a string passed to a call fails", () => {
      const findings = scan(vue(`globalThis.prompt("Block reason:");`));
      expect(findings.join("\n")).toContain("Block reason:");
    });

    it("a backtick sentence in an interpolation fails", () => {
      const findings = scan(vue("", "{{ `Sign in with ${provider}` }}"));
      expect(findings.join("\n")).toContain("Sign in with");
    });

    it("a plain `.ts` file is scanned too", () => {
      const findings = scan(`const q = { error: "State mismatch — try again." };`, false);
      expect(findings).toContain('error: "State mismatch — try again."');
    });
  });

  describe("stays quiet about text that is not prose", () => {
    it("a cva variant map of Tailwind classes passes", () => {
      const findings = scan(
        vue(`const v = { outline: "border border-primary/40 bg-transparent hover:bg-accent" };`),
      );
      expect(findings).toEqual([]);
    });

    it("a hyphenated sentence is not mistaken for a class list", () => {
      // The counter-case to the one above: all-lowercase tokens with a single
      // hyphen between them is prose, and a rule that demanded *every* token
      // carry a separator would have swallowed it.
      const findings = scan(vue(`row.error = "reason is required (set per-row or default)";`));
      expect(findings).toContain('row.error = "reason is required (set per-row or default)"');
    });

    it("a defineModel name passes", () => {
      const findings = scan(vue(`const open = defineModel<boolean>("mobileOpen");`));
      expect(findings).toEqual([]);
    });

    it("an HTTP header value passes", () => {
      const findings = scan(vue("const h = {}; h[\"Authorization\"] = `Bearer ${token}`;"));
      expect(findings).toEqual([]);
    });

    it("a console message passes — it reaches a log, not a screen", () => {
      const findings = scan(vue(`console.error("Failed to refresh the token.");`));
      expect(findings).toEqual([]);
    });

    it("a catalogue key passed to t() passes", () => {
      const findings = scan(vue(`const label = t("adminHealth.clearCache");`));
      expect(findings).toEqual([]);
    });

    it("a CLI invocation under a `code:` key passes", () => {
      const findings = scan(vue(`const s = { code: "batlehub-cli registry list" };`));
      expect(findings).toEqual([]);
    });

    it("a class list bound with :class passes", () => {
      const findings = scan(vue("", `<div :class="ok ? 'text-foreground' : 'text-destructive'" />`));
      expect(findings).toEqual([]);
    });
  });
});
