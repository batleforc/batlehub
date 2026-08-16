/**
 * What counts as a translatable string, shared by the audit and the extractor.
 *
 * These lived in the extractor only, which meant the audit kept *counting*
 * things the extractor correctly refused to touch — a gap that can never reach
 * zero however much work you do. One definition, both tools.
 */

/** Tokens rather than sentences: numbers, punctuation, entities. */
export const NOT_PROSE = [
  /^[\s\d.,:;/|—–-]*$/,
  /^[{}[\]()<>«»"'`]+$/,
  /^&[a-z]+;$/i,
];

/**
 * Product, registry and tool names that §4.6 keeps verbatim in both locales.
 * They are capitalised exactly like a label, so no rule about *shape* can tell
 * them apart from one — they have to be named.
 */
export const DOMAIN_NOUNS = new Set([
  "BatleHub",
  "GitHub",
  "GitLab",
  "Forgejo",
  "JetBrains",
  "NuGet",
  "Maven",
  "Terraform",
  "RubyGems",
  "Composer",
  "Conda",
  "PyPI",
  "OpenVSX",
  "Cargo",
  "Docker",
  "Helm",
  "Debian",
  "Homebrew",
  "CocoaPods",
  "Swift",
  "Nix",
  "Go",
  "Windows",
  "Linux",
  "macOS",
]);

/**
 * The same rule for terms that contain a space, which `DOMAIN_NOUNS` cannot
 * reach because it is only consulted for bare words. Platform and target names
 * are read the same way in both locales; translating them would name a
 * download that does not exist.
 */
export const DOMAIN_PHRASES = new Set([
  "Linux x86_64",
  "Linux aarch64",
  "macOS Apple Silicon",
  "macOS Intel",
]);

/**
 * A bare word has no sentence around it to judge it by, so it is classified by
 * shape. `latest` and `yank` are domain terms §4.6 keeps verbatim, `GET` is an
 * HTTP verb, `NuGet` is a product name — but `Registries` and `You` are labels
 * somebody has to translate.
 *
 * This replaces a single `/^[a-z0-9_.-]+$/i`, whose `i` flag made *every*
 * unspaced word an identifier. That is why `Registries` and `You` sat in
 * `HomePage.vue` while the gate reported zero: a rule that cannot fail is not a
 * gate, which is the same lesson R11a records about the colour tokens.
 */
const isBareWordNotProse = (text) => {
  if (/\s/.test(text)) return false; // has a space: judge it as prose
  if (/[_./:@\d-]/.test(text)) return true; // identifier: foo_bar, v1.2, owner/repo, Content-Disposition
  if (/^[a-z]+[A-Z]/.test(text)) return true; // camelCase: mobileOpen, registryName
  if (text === text.toLowerCase()) return true; // domain term: latest, yank, npm
  if (text === text.toUpperCase()) return true; // verb or acronym: GET, SBOM, API
  return DOMAIN_NOUNS.has(text); // product name, or else a real label
};

/**
 * Format examples and config identifiers. A user *types* these; translating
 * them would make them wrong, so they do not belong in a catalogue at all.
 */
export const isSample = (text) =>
  /^e\.?g\.?[\s.]/i.test(text) ||
  /CVE-\d{4}/.test(text) ||
  /&#\d+;/.test(text) ||
  (!text.includes(" ") && /[/:,@]/.test(text)) || // owner/repo, latest_n:, oidc1:team-a
  /^[\w.-]+(,[\w.-]*){2,}/.test(text); // a CSV row

/**
 * A Tailwind class list, which is markup rather than text.
 *
 * The template scan skips these by skipping `:class` — an attribute name it can
 * see. A `cva()` variant map has no attribute to skip: the class list is the
 * *value* of a key called `default` or `destructive`, and it outnumbers the real
 * findings roughly twenty to one, which would bury this gate rather than
 * sharpen it.
 *
 * Judged by shape, since that is all there is. Two conditions, and the second
 * is a *density* rather than a rule about every token, because `border` and
 * `underline` are real utility classes with no separator in them:
 *
 *   1. every token is class-shaped — lowercase or bracketed, no sentence
 *      punctuation. "Theme: follow system" fails on the capital `T`.
 *   2. at least 70% of them carry a `-`, `:`, `/` or `[`.
 *
 * A bare `every` on (2) would have to let `set per-row or use default reason`
 * through as a class list, which is prose with one hyphen in it — a translated
 * string lost to a rule that was too eager, and the opposite of this gate's job.
 */
export const isClassList = (text) => {
  const tokens = text.split(/\s+/).filter(Boolean);
  if (tokens.length < 2) return false;
  // `var(--action-ring)` and `transition-[box-shadow,background-color]` are
  // single classes, so parens and commas are part of the alphabet.
  if (!tokens.every((t) => /^[a-z0-9[!]-?[\w:/.%,()[\]&>~*+-]*$/.test(t))) return false;
  return tokens.filter((t) => /[-:/[]/.test(t)).length / tokens.length >= 0.7;
};

/**
 * Prose needs at least two consecutive letters. Stripping an interpolation can
 * leave a residue that looks like text but is a unit or a bracket around a
 * value — `({{ days }}d)` becomes `(d)`, which is not a string anyone wrote.
 */
export const isProse = (text) =>
  text.length >= 3 &&
  /[a-z]{2}/i.test(text) &&
  !NOT_PROSE.some((re) => re.test(text)) &&
  !isBareWordNotProse(text);

/**
 * Everything the two tools agree is a user-visible string.
 *
 * A trailing colon is stripped before classification. `Source:` is a field
 * label; `latest_n:` and `oidc1:team-a` are identifiers, and they stay
 * identifiers after the strip because of the `_` and the interior `:`. Without
 * this, `isSample`'s "no space and contains a separator" clause read every
 * `<strong>Created:</strong>` in the console as a config key — which is how two
 * of them sat untranslated on the config-reload page.
 */
export const isTranslatable = (raw) => {
  const text = raw.replace(/:\s*$/, "");
  return isProse(text) && !isSample(text) && !isClassList(text) && !DOMAIN_PHRASES.has(text);
};

/** The catalogue holds text, not markup: vue-i18n renders `&amp;` literally. */
const ENTITIES = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&quot;": '"',
  "&#39;": "'",
  "&nbsp;": " ",
};

export const decode = (text) =>
  text.replace(/&(amp|lt|gt|quot|#39|nbsp);/g, (m) => ENTITIES[m] ?? m);
