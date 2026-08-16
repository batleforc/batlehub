import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import RichText from "./RichText.vue";
import { REGISTRY_TYPE_DEFS, type SnippetContext } from "@/config/registryTypes";
import { REGISTRY_PATH_TYPES } from "@/config/registryPathFields";

const render = (markup: string, codeClass?: string) =>
  mount(RichText, { props: codeClass ? { markup, codeClass } : { markup } });

describe("RichText", () => {
  it("renders the allowlisted tags as real elements", () => {
    const w = render("Set <code>GOPROXY</code>, then <strong>save</strong> the <em>file</em>.");
    expect(w.find("code").text()).toBe("GOPROXY");
    expect(w.find("strong").text()).toBe("save");
    expect(w.find("em").text()).toBe("file");
    expect(w.text()).toBe("Set GOPROXY, then save the file.");
  });

  it("styles code with the class the call site asks for", () => {
    expect(render("<code>x</code>").find("code").classes()).toContain("font-mono");
    expect(render("<code>x</code>", "text-xs font-mono").find("code").classes()).toContain(
      "text-xs",
    );
  });

  it("opens an external link safely", () => {
    const a = render('See <a href="https://pypi.org">PyPI</a>.').find("a");
    expect(a.attributes("href")).toBe("https://pypi.org");
    expect(a.attributes("rel")).toBe("noopener noreferrer");
    expect(a.attributes("target")).toBe("_blank");
  });

  /**
   * `ATTRS` admits all three spellings, so `safeHref` has to read all three.
   * Reading fewer parses the tag as a link, finds no href, drops it, and leaves
   * the label as bare text with nothing to show a link was meant to be there.
   */
  it("reads every href spelling the tag grammar admits", () => {
    for (const attr of [
      `href="https://pypi.org"`,
      `href='https://pypi.org'`,
      `href=https://pypi.org`,
    ]) {
      const a = render(`See <a ${attr}>PyPI</a>.`).find("a");
      expect(a.exists(), attr).toBe(true);
      expect(a.attributes("href"), attr).toBe("https://pypi.org");
    }
  });

  /**
   * The attribute name is anchored. Unanchored, `href\s*=` also matches the
   * tail of `data-href=`, so a decoy would win over the author's real href and
   * the rendered link would not be the one written in the source.
   */
  it("ignores an attribute that merely ends in href", () => {
    const a = render(
      '<a data-href="https://phish.example" href="https://pypi.org">Read the docs</a>',
    ).find("a");
    expect(a.attributes("href")).toBe("https://pypi.org");
  });

  /**
   * The finding this component exists to close. `registryUrl` and
   * `registryName` are built from a registry name that comes back from the API,
   * so the markup these notes produce is not fully static — under `v-html` an
   * admin-chosen name was a stored-XSS payload on the Setup Guide.
   */
  it("shows injected markup instead of running it", () => {
    const evil = `<img src=x onerror="alert(1)"><script>alert(2)</script>`;
    const w = render(`Point at <code>${evil}</code> to publish.`);
    expect(w.find("img").exists()).toBe(false);
    expect(w.find("script").exists()).toBe(false);
    expect(w.find("code").text()).toBe(evil);
  });

  /**
   * Only an absolute `http(s)` URL becomes a link.
   *
   * `/\evil.example` is the one that motivates the rule: WHATWG folds `\` into
   * `/` for special schemes, so it resolves to `https://evil.example/`. A guard
   * that admitted same-origin paths and merely rejected a leading `//` classed
   * that as an internal route and shipped it without `rel`.
   */
  it("refuses every href that is not an absolute http(s) URL", () => {
    const refused = [
      "javascript:alert(1)",
      "data:text/html,<script>",
      "//evil.example",
      String.raw`/\evil.example`,
      String.raw`/\/evil.example`,
      "/setup#npm",
      "#anchor",
      "mailto:security@example.test",
    ];
    for (const href of refused) {
      const w = render(`<a href="${href}">click</a>`);
      expect(w.find("a").exists(), href).toBe(false);
      expect(w.text(), href).toBe("click");
    }
  });

  it("decodes entities once, so an escaped entity stays escaped", () => {
    expect(render("a &amp;&amp; b").text()).toBe("a && b");
    expect(render("<code>&lt;repo&gt;.db</code>").find("code").text()).toBe("<repo>.db");
    expect(render("&amp;lt;").text()).toBe("&lt;");
  });

  /**
   * `v-html` decoded every HTML entity, so anything this does not decode is a
   * regression against what it replaced — the raw `&rsquo;` printed at the
   * reader. Curly quotes especially: they are what prose reaches for.
   */
  it("decodes the punctuation and numeric entities prose uses", () => {
    expect(render("proxy &mdash; hybrid").text()).toBe("proxy — hybrid");
    expect(render("wait&hellip;").text()).toBe("wait…");
    expect(render("it&rsquo;s &ldquo;cached&rdquo;").text()).toBe("it’s “cached”");
    expect(render("a &#8212; b").text()).toBe("a — b");
    expect(render("it&#x27;s").text()).toBe("it's");
    expect(render("a&nbsp;b").text()).toBe("a b");
    expect(render("&notanentity; stays").text()).toBe("&notanentity; stays");
  });

  /**
   * Every string the two data files hand this component, rendered.
   *
   * Three properties at once, because each is a way the component can quietly
   * render less than the `v-html` it replaced: an unknown *tag* would print as
   * escaped source, an unknown *entity* would print as `&rsquo;`, and a link the
   * href reader cannot parse would vanish into bare text. None of the three is
   * visible from the component's own unit tests — only from the corpus.
   *
   * Deduplicated: static notes are identical across contexts, and mounting ~170
   * strings twice put this test within reach of vitest's 5 s default under
   * full-suite parallel load.
   */
  it("renders every registry description and note without loss", { timeout: 20_000 }, () => {
    // Several notes branch on the context — `isPublishMode(ctx)` in the apt
    // and rpm entries, `ctx.isAuthenticated` in both VSCodium ones. One
    // context leaves the other branch unread, so markup added there would
    // slip past this assertion entirely.
    const contexts: SnippetContext[] = (["local", "proxy"] as const).map((mode, i) => ({
      base: "https://example.test",
      registryName: "hub",
      registryUrl: "https://example.test/proxy/hub",
      urlFor: (name: string) => `https://example.test/proxy/${name}`,
      mode,
      isAuthenticated: i === 0,
      token: "tok",
      netrcHost: "example.test",
      netrcLogin: "user",
      identity: null,
      selectedNames: {},
    }));

    const markup = new Set(
      [
        ...contexts.flatMap((ctx) =>
          REGISTRY_TYPE_DEFS.flatMap((def) => [
            def.description,
            ...def.snippets.map((s) =>
              typeof s.note === "function" ? s.note(ctx) : (s.note ?? ""),
            ),
          ]),
        ),
        ...REGISTRY_PATH_TYPES.map((def) => def.note ?? ""),
      ].filter(Boolean),
    );

    const unknownTags = new Set<string>();
    for (const one of markup) {
      for (const [tag] of one.matchAll(/<\/?([a-z][a-z0-9]*)\b/gi)) {
        const name = tag.replace(/^<\/?/, "").toLowerCase();
        if (!["code", "strong", "em", "a"].includes(name)) unknownTags.add(name);
      }

      const rendered = render(one);
      const text = rendered.text();
      // Nothing the parser left behind may still look like markup — this is
      // also what catches a nested tag, whose inner markup lands in the outer
      // tag's text.
      expect(text, one).not.toMatch(/<(code|strong|em|a)\b/i);
      // Every entity the corpus uses has to be in ENTITIES.
      expect(text, one).not.toMatch(/&(?:[a-z]+|#\d+|#x[0-9a-f]+);/i);
      // Every anchor written has to survive as an anchor.
      expect(rendered.findAll("a").length, one).toBe((one.match(/<a\b/gi) ?? []).length);
    }
    expect([...unknownTags], "tags outside RichText's grammar").toEqual([]);
  });
});
