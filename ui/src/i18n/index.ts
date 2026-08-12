import { createI18n } from "vue-i18n";

import en from "@/locales/en.json";
import fr from "@/locales/fr.json";

/**
 * The translation layer (RFC 0003 §4.6).
 *
 * Two rules shape everything here, and both exist because getting them wrong is
 * expensive later rather than visibly broken now:
 *
 *  1. **The stored preference and the resolved locale are different things.**
 *     The preference is `system | en | fr`; the resolved locale is what renders.
 *     Storing `en` when the user meant "follow my browser" silently swallows
 *     every later change to their browser language. Same pattern as the theme.
 *
 *  2. **Domain terms are not translated.** `yank`, `latest`, the registry modes
 *     (`proxy`/`local`/`hybrid`), config keys and CLI invocations stay verbatim
 *     in both catalogues. A French UI that says *chaîne de caractères de
 *     version* where the config says `version_pattern` is worse than English:
 *     the user cannot search for it, cannot type it, and cannot match it against
 *     the docs. Translate the sentence around the term, never the term.
 */

export const LOCALE_KEY = "batlehub.locale";

export type LocalePreference = "system" | "en" | "fr";
export type Locale = "en" | "fr";

export const SUPPORTED: readonly Locale[] = ["en", "fr"] as const;

/** What the browser asks for, narrowed to what we actually have. */
export function systemLocale(): Locale {
  const tag = (globalThis.navigator?.language ?? "en").toLowerCase();
  return tag.startsWith("fr") ? "fr" : "en";
}

export function readPreference(): LocalePreference {
  try {
    const stored = localStorage.getItem(LOCALE_KEY);
    return stored === "en" || stored === "fr" || stored === "system" ? stored : "system";
  } catch {
    return "system"; // private mode, or storage disabled
  }
}

export function resolveLocale(preference: LocalePreference): Locale {
  return preference === "system" ? systemLocale() : preference;
}

export const i18n = createI18n({
  legacy: false,
  locale: resolveLocale(readPreference()),
  fallbackLocale: "en",
  messages: { en, fr },
  /* A missing key is a bug, not a runtime decision. It falls back to English so
     the user still reads a sentence, and warns loudly in development so the gap
     is found before a release rather than by a French speaker. */
  missingWarn: import.meta.env.DEV,
  fallbackWarn: import.meta.env.DEV,
});

/**
 * Apply a preference: persist it, switch the catalogue, and update `lang` on
 * the document — which was a hardcoded attribute in `index.html`, so assistive
 * tech and browser translation prompts were told every page was English.
 */
export function setLocalePreference(preference: LocalePreference): Locale {
  try {
    localStorage.setItem(LOCALE_KEY, preference);
  } catch {
    /* session-only is an acceptable degradation */
  }
  const resolved = resolveLocale(preference);
  i18n.global.locale.value = resolved;
  document.documentElement.setAttribute("lang", resolved);
  return resolved;
}

/** Call once at boot, so `lang` matches what is actually rendered. */
export function initLocale(): void {
  document.documentElement.setAttribute("lang", resolveLocale(readPreference()));
}
