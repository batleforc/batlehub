<script setup lang="ts">
/**
 * Theme as a stored preference, not a two-way switch.
 *
 * The stored value is `system | light | dark`; the *resolved* value is what
 * renders. They are deliberately separate: storing "dark" when the user meant
 * "follow my device" silently swallows every later change the OS makes, which
 * is precisely what a two-state toggle cannot express (RFC 0003 R12).
 *
 * `data-theme` on <html> is what the token layer keys off — see
 * ui/src/design/tokens.css. The same three-state pattern backs the locale.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useColorMode } from "@vueuse/core";
import { Sun, Moon, Monitor } from "@lucide/vue";
import { Button } from "@/components/ui/button";

const { t } = useI18n();

const mode = useColorMode({
  attribute: "data-theme",
  storageKey: "batlehub.theme",
  emitAuto: true,
});

const ORDER = ["auto", "light", "dark"] as const;

/**
 * `auto` is vueuse's name for the stored preference; `theme.system` is the
 * catalogue's. Mapped rather than renamed because `emitAuto` decides the former
 * and `catalogues.test.ts` decides the latter.
 *
 * These three keys were translated, correct, and referenced zero times while
 * this map held the English — RFC 0004-bis §2.2, from the inside.
 */
const LABEL_KEYS: Record<string, string> = {
  auto: "theme.system",
  light: "theme.light",
  dark: "theme.dark",
};

/** Keyed the same way as `LABEL_KEYS`, and for the same reason. */
const ICONS: Record<string, typeof Monitor> = {
  auto: Monitor,
  light: Sun,
  dark: Moon,
};

const icon = computed(() => ICONS[mode.value] ?? ICONS.auto);
const label = computed(() => t(LABEL_KEYS[mode.value] ?? LABEL_KEYS.auto));

function cycle(): void {
  const index = ORDER.indexOf(mode.value as (typeof ORDER)[number]);
  mode.value = ORDER[(index + 1) % ORDER.length];
}
</script>

<template>
  <Button variant="ghost" size="icon" :title="label" :aria-label="label" @click="cycle()">
    <component :is="icon" class="h-4 w-4" />
  </Button>
</template>
