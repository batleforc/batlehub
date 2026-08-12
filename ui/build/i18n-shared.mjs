/**
 * What counts as a translatable string, shared by the audit and the extractor.
 *
 * These lived in the extractor only, which meant the audit kept *counting*
 * things the extractor correctly refused to touch — a gap that can never reach
 * zero however much work you do. One definition, both tools.
 */

/** Tokens rather than sentences: identifiers, numbers, punctuation, entities. */
export const NOT_PROSE = [
  /^[\s\d.,:;/|—–-]*$/,
  /^[a-z0-9_.-]+$/i,
  /^[{}[\]()<>«»"'`]+$/,
  /^&[a-z]+;$/i,
];

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
 * Prose needs at least two consecutive letters. Stripping an interpolation can
 * leave a residue that looks like text but is a unit or a bracket around a
 * value — `({{ days }}d)` becomes `(d)`, which is not a string anyone wrote.
 */
export const isProse = (text) =>
  text.length >= 3 && /[a-z]{2}/i.test(text) && !NOT_PROSE.some((re) => re.test(text));

/** Everything the two tools agree is a user-visible string. */
export const isTranslatable = (text) => isProse(text) && !isSample(text);

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
