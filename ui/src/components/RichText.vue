<script setup lang="ts">
import { computed } from "vue";

/**
 * Renders the small trusted markup that `registryTypes.ts` and
 * `registryPathFields.ts` carry — without `v-html`.
 *
 * Those strings are not wholly static: a note interpolates `ctx.registryUrl`
 * and `ctx.registryName`, both built from a registry name that arrives from the
 * API. Handing the result to `v-html` therefore made an admin-chosen registry
 * name a stored-XSS vector on the Setup Guide, which is what code scanning
 * flagged.
 *
 * Here the string is tokenised against a closed grammar — `code`, `strong`,
 * `em`, `a href` and nothing else — and every fragment is emitted as a *text*
 * interpolation, which Vue escapes. A registry name carrying an inline event
 * handler (an `onerror` on an image, say) is shown, not run, and no new tag can
 * be smuggled in by writing one: the grammar has no branch that recognises it.
 *
 * The boundary this draws is *script execution*, not *authorship*. A registry
 * name is still interpolated into markup, so one containing an anchor to an
 * attacker's host renders as a real link, and `strong` can still forge
 * emphasis. That is accepted: the name comes from `config.toml`, so anyone who
 * can set it already owns the deployment. Nothing here should be relied on to
 * sanitise input from a lower trust level than the operator's.
 *
 * Two deliberate limits, both gated by the corpus test rather than left to
 * memory: the grammar is *flat* (a tag inside a tag renders its inner markup as
 * visible text), and only absolute `http(s)` links are honoured. A relative
 * href would need `RouterLink` to avoid a full page load, and this component
 * has no router; rendering a bare anchor instead would silently drop SPA state.
 *
 * Styling lives here rather than in the data, so the two files hold markup and
 * not Tailwind. `codeClass` is the one knob, because a card description sets
 * its own type scale and a note inherits one.
 */
const props = withDefaults(
  defineProps<{
    markup: string;
    /** Classes for `<code>` spans. Descriptions pass the `text-xs` variant. */
    codeClass?: string;
  }>(),
  { codeClass: "font-mono bg-muted px-1 rounded-sm" },
);

const LINK_CLASS = "underline underline-offset-2 hover:text-foreground transition-colors";

type Part =
  | { kind: "text" | "code" | "strong" | "em"; text: string }
  | { kind: "link"; text: string; href: string };

/**
 * `<code>`, `<strong>`, `<em>`, or an `<a>` whose attributes are read below.
 *
 * The attribute run steps over quoted values rather than stopping at the first
 * `>`, so a `>` inside one — `href="data:text/html,<script>"` — closes nothing.
 * A naive `[^>]*` ends the tag mid-attribute and spills the remainder into the
 * text, which is escaped but reads as garbage on the page.
 */
const ATTRS = String.raw`(?:"[^"]*"|'[^']*'|[^>"'])*`;
const TAG = new RegExp(
  String.raw`<(code|strong|em)\b${ATTRS}>([\s\S]*?)<\/\1\s*>|<a\b(${ATTRS})>([\s\S]*?)<\/a\s*>`,
  "gi",
);

/**
 * The `href`, in any of the three spellings `ATTRS` admits.
 *
 * The name is anchored to a boundary: an unanchored `href\s*=` also matches the
 * tail of `data-href=`, so a decoy attribute would win over the author's real
 * one and the rendered link would not be the link in the source. All three
 * value forms are read because `ATTRS` accepts all three — matching fewer would
 * parse the tag as a link, find no href, and drop it, leaving the label as bare
 * text with nothing to show a link was ever meant to be there.
 */
const HREF = /(?:^|[\s"'])href\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i;

/**
 * Named entities. Numeric ones are decoded generically below, so this carries
 * names only.
 *
 * `v-html` decoded the whole HTML set; this decodes what it knows and leaves
 * the rest verbatim, which is the one way this component can render *less* than
 * what it replaced. The corpus test asserts no undecoded entity survives, so
 * this map is a contract rather than a hope — a note reaching for a name that
 * is not here fails the suite instead of printing `&rsquo;` at the reader.
 */
const ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  // Written as an escape, not the character: a raw U+00A0 in source is
  // indistinguishable from a plain space, so the one entity whose whole
  // point is that it does not break would be a space nobody could review.
  nbsp: "\u00a0",
  mdash: "—",
  ndash: "–",
  hellip: "…",
  lsquo: "\u2018",
  rsquo: "\u2019",
  ldquo: "“",
  rdquo: "”",
  laquo: "«",
  raquo: "»",
  rarr: "→",
  larr: "←",
  times: "×",
  middot: "·",
  bull: "•",
  deg: "°",
  plusmn: "±",
  copy: "©",
  reg: "®",
  trade: "™",
  euro: "€",
};

/** One pass, so `&amp;lt;` decodes to the literal `&lt;` rather than to `<`. */
const decode = (text: string) =>
  text.replace(/&(#\d+|#x[0-9a-f]+|\w+);/gi, (whole, name: string) => {
    if (!name.startsWith("#")) return ENTITIES[name.toLowerCase()] ?? whole;
    const hex = name[1]?.toLowerCase() === "x";
    const code = hex ? parseInt(name.slice(2), 16) : Number(name.slice(1));
    return Number.isInteger(code) && code > 0 && code <= 0x10ffff
      ? String.fromCodePoint(code)
      : whole;
  });

/**
 * Absolute `http(s)` only. Everything else — `javascript:`, `data:`, a
 * protocol-relative `//evil.example`, a relative path — renders as the link's
 * text instead.
 *
 * Requiring an absolute URL is what makes the check total. An earlier version
 * admitted same-origin paths and rejected `//`, which the URL parser walks
 * straight around: WHATWG folds `\` into `/` for special schemes, so
 * `/\evil.example` resolves to `https://evil.example/` — an off-origin
 * navigation classed as an internal route, and shipped without `rel` because it
 * had been judged same-origin. Matching a scheme leaves nothing to fold.
 */
function safeHref(attrs: string): string | null {
  const match = HREF.exec(attrs);
  const raw = match?.[1] ?? match?.[2] ?? match?.[3];
  if (!raw) return null;
  const href = decode(raw).trim();
  return /^https?:\/\/[^\s/\\]/i.test(href) ? href : null;
}

const parts = computed<Part[]>(() => {
  const out: Part[] = [];
  const src = props.markup ?? "";
  let cursor = 0;

  // `matchAll` over `exec`: it iterates a clone, so the module-level `TAG`
  // keeps no `lastIndex` for a second component instance to resume from.
  for (const match of src.matchAll(TAG)) {
    const at = match.index ?? 0;
    if (at > cursor) out.push({ kind: "text", text: decode(src.slice(cursor, at)) });
    cursor = at + match[0].length;

    if (match[1]) {
      out.push({
        kind: match[1].toLowerCase() as "code" | "strong" | "em",
        text: decode(match[2]),
      });
      continue;
    }
    const href = safeHref(match[3] ?? "");
    const text = decode(match[4]);
    out.push(href ? { kind: "link", text, href } : { kind: "text", text });
  }

  if (cursor < src.length) out.push({ kind: "text", text: decode(src.slice(cursor)) });
  return out;
});
</script>

<template>
  <span>
    <template v-for="(part, i) in parts" :key="i">
      <code v-if="part.kind === 'code'" :class="codeClass">{{ part.text }}</code>
      <strong v-else-if="part.kind === 'strong'">{{ part.text }}</strong>
      <em v-else-if="part.kind === 'em'">{{ part.text }}</em>
      <a
        v-else-if="part.kind === 'link'"
        :href="part.href"
        :class="LINK_CLASS"
        target="_blank"
        rel="noopener noreferrer"
        >{{ part.text }}</a
      >
      <template v-else>{{ part.text }}</template>
    </template>
  </span>
</template>
