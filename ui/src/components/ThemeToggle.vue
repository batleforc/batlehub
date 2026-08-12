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
import { useColorMode } from "@vueuse/core";
import { Sun, Moon, Monitor } from "@lucide/vue";
import { Button } from "@/components/ui/button";

const mode = useColorMode({
  attribute: "data-theme",
  storageKey: "batlehub.theme",
  emitAuto: true,
});

const ORDER = ["auto", "light", "dark"] as const;

const LABELS: Record<string, string> = {
  auto: "Theme: follow system",
  light: "Theme: light",
  dark: "Theme: dark",
};

const icon = computed(() => (mode.value === "auto" ? Monitor : mode.value === "dark" ? Moon : Sun));
const label = computed(() => LABELS[mode.value] ?? LABELS.auto);

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
