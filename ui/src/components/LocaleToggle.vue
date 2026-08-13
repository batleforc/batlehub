<script setup lang="ts">
/**
 * Language as a stored preference, mirroring the theme control exactly.
 *
 * Three states, because "follow my browser" is a real answer that a two-way
 * switch cannot express — and storing the *resolved* locale instead would
 * silently swallow a later change to the browser's language (RFC 0003 §4.6).
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Languages } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { readPreference, setLocalePreference, systemLocale, type LocalePreference } from "@/i18n";

const { t } = useI18n();
const preference = ref<LocalePreference>(readPreference());

const ORDER: LocalePreference[] = ["system", "en", "fr"];

/* Spelled out rather than built as `t(\`locale.${preference.value}\`)`. A
   template-literal key is invisible to the catalogue's reference gate, which is
   how 94 keys drifted out of use with everything green (RFC 0004-bis §2.2), and
   invisible to anyone grepping for the key they are about to delete. */
const NAME_KEYS: Record<Exclude<LocalePreference, "system">, string> = {
  en: "locale.en",
  fr: "locale.fr",
};

const label = computed(() => {
  const name =
    preference.value === "system"
      ? `${t("locale.system")} (${systemLocale().toUpperCase()})`
      : t(NAME_KEYS[preference.value]);
  return `${t("locale.label")}: ${name}`;
});

function cycle(): void {
  const next = ORDER[(ORDER.indexOf(preference.value) + 1) % ORDER.length];
  preference.value = next;
  setLocalePreference(next);
}
</script>

<template>
  <Button variant="ghost" size="icon" :title="label" :aria-label="label" @click="cycle()">
    <Languages class="h-4 w-4" />
  </Button>
</template>
