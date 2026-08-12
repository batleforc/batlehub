import type { Visibility } from "@/client/types.gen";

export type { Visibility };

/**
 * The visibility levels, in the order a picker offers them: widest first.
 *
 * This is UI configuration, not a mirror of a response — `label` is an i18n key
 * the consuming component resolves, and the server has no opinion about either
 * the order or the wording. The `Visibility` union itself comes from the
 * generated client (RFC 0004 §4.1); only this list lives here.
 */
export const VISIBILITY_OPTIONS = [
  { value: "public" as Visibility, label: "visibility.public" },
  { value: "internal" as Visibility, label: "visibility.internal" },
  { value: "team" as Visibility, label: "visibility.team" },
] as const;
