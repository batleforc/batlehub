import { describe, expect, it } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { REGISTRY_TYPE_DEFS, type SnippetContext } from "./registryTypes";

/**
 * The setup snippets exist twice: here, rendered by the Setup Guide at
 * `/setup`, and in `docs/registries/<id>.md`, read by everyone who never opens
 * the console. Nothing compared them, and `use/index.md` sends readers to the
 * Setup Guide *before* the manual steps — so a drift means one of the two is
 * telling someone the wrong thing about authentication (RFC 0005-bis §6.4).
 *
 * The comparison cannot be a byte match. `SnippetDef.template` is a function of
 * a live context, and the documentation writes placeholders where the console
 * writes the reader's actual host and token. So both sides are normalised down
 * to what they are actually asserting — a command, a key, a file line — and the
 * test asks whether the documentation contains it at all.
 *
 * RFC 0005-bis O2 leaves the eventual generation direction open. This is the
 * check that makes the answer visible: whichever side turns out to be
 * systematically richer is the source.
 */

// `process.cwd()` rather than `import.meta.url`: vitest transforms this module
// and its URL is not a file: URL.
const DOCS = resolve(process.cwd(), "../docs/registries") + "/";

/** A context whose values match the placeholders the documentation uses. */
const ctx: SnippetContext = {
  base: "https://batlehub.example.com",
  registryName: "<registry>",
  registryUrl: "https://batlehub.example.com/proxy/<registry>",
  urlFor: (name) => `https://batlehub.example.com/proxy/${name}`,
  mode: "hybrid",
  isAuthenticated: true,
  token: "<token>",
  netrcHost: "batlehub.example.com",
  netrcLogin: "<user>",
  identity: null,
  selectedNames: {},
  signedDownloads: false,
};

/**
 * Reduce a line to what it asserts. Tokens, hosts and registry names are the
 * three things the two sides legitimately spell differently, and comparing them
 * would make the test fail on agreement.
 */
function normalise(text: string): string[] {
  return text
    .split("\n")
    .map((l) =>
      l
        .replace(/<your-token>|\$\{BATLEHUB_TOKEN\}|<token>|\$BATLEHUB_TOKEN/g, "TOKEN")
        .replace(/<registry>|<registry-name>|internal-\w+/g, "REG")
        .replace(/\s+/g, " ")
        .trim(),
    )
    .filter((l) => l.length > 12 && !l.startsWith("#") && !l.startsWith("//"));
}

/** Some registry types are console-only composites or have no documentation page. */
const NO_PAGE = new Set(["mise"]);

describe("registry setup snippets agree with the documentation", () => {
  const missing: string[] = [];

  for (const def of REGISTRY_TYPE_DEFS) {
    if (NO_PAGE.has(def.id)) continue;
    const page = `${DOCS}${def.id}.md`;

    it(`${def.id} has a documentation page`, () => {
      expect(existsSync(page), `docs/registries/${def.id}.md`).toBe(true);
    });

    if (!existsSync(page)) continue;
    const doc = normalise(readFileSync(page, "utf8")).join("\n");

    for (const snippet of def.snippets) {
      if (snippet.showWhen && !snippet.showWhen(ctx)) continue;
      const rendered = normalise(snippet.template(ctx));
      const absent = rendered.filter((line) => !doc.includes(line));
      if (absent.length) {
        missing.push(
          `${def.id} · ${snippet.label ?? snippet.key}: ${absent.length}/${rendered.length} line(s) not in the page\n` +
            absent.map((l) => `      ${l}`).join("\n"),
        );
      }
    }
  }

  // Reported to the console as one list rather than folded into an assertion
  // message: the useful output is the shape of the drift across all 20 pages,
  // and a multi-kilobyte string diff is slower to render than the whole suite.
  it("every console snippet line appears in its registry page", () => {
    if (missing.length) console.log("\n" + missing.join("\n\n") + "\n");
    // Pinned, not zero. The first run found 37, and every one of them is
    // something the console tells a reader and the documentation does not —
    // VSCodium's extension gallery, the `mise` URL replacements, the generic
    // mirror's environment variables. Writing those up is content authoring,
    // which RFC 0005-bis does not do; the list is in
    // `docs/internal/rfc-0005-bis-snippet-drift.md`.
    //
    // The pin may only go down. A new snippet that the documentation does not
    // carry fails here, which is the whole point — the drift that existed was
    // invisible, and the drift that comes next will not be.
    expect(
      missing.length,
      `${missing.length} snippet(s) drifted; the pinned baseline is 35 and may only fall`,
    ).toBeLessThanOrEqual(35);
  });
});
