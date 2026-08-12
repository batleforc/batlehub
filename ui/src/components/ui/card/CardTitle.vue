<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

/**
 * The heading level is a prop because it is a property of where the card sits,
 * not of what a card is. It was hardcoded `h3`, so every page whose title is an
 * `h1` and whose first card follows it skipped `h2` — `/tools` renders
 * "Tools" then "Access Check" with nothing between them, which is a broken
 * document outline for anyone navigating by heading.
 *
 * `h2` is the default because directly under the page `h1` is where cards
 * mostly are; pass `as="h3"` for one nested under a section heading.
 */
const props = withDefaults(
  defineProps<{ class?: HTMLAttributes["class"]; as?: "h2" | "h3" | "h4" }>(),
  { as: "h2" },
);
</script>

<template>
  <component
    :is="as"
    :class="cn('font-mono text-2xl font-bold leading-none tracking-tight', props.class)"
  >
    <slot />
  </component>
</template>
