import type { Visibility } from "@/client/types.gen";

export type { Visibility };

/**
 * The visibility levels, in the order a picker offers them: widest first.
 *
 * This is UI configuration, not a mirror of a response — `label` is an i18n key
 * the consuming component resolves, and the server has no opinion about either
 * the order or the wording. The `Visibility` union itself comes from the
 * generated client (RFC 0004 §4.1); only this list lives here.
 *
 * `shortLabel` is its own message rather than the long one cut at the dash:
 * French does not put the dash where English does. It lives here, spelled out,
 * rather than being built as `` t(`visibilityShort.${value}`) `` at the one call
 * site — a template-literal key is invisible to the catalogue's reference gate
 * (RFC 0004-bis §4.2) and to anyone grepping before they delete it.
 */
export const VISIBILITY_OPTIONS = [
  {
    value: "public" as Visibility,
    label: "visibility.public",
    shortLabel: "visibilityShort.public",
  },
  {
    value: "internal" as Visibility,
    label: "visibility.internal",
    shortLabel: "visibilityShort.internal",
  },
  { value: "team" as Visibility, label: "visibility.team", shortLabel: "visibilityShort.team" },
] as const;
