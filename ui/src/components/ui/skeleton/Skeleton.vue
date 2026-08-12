<script setup lang="ts">
/**
 * A loading placeholder shaped like the content it stands in for.
 *
 * Deliberately not a shimmer sweep: a moving gradient is one of the category's
 * tells, and it animates a region the user is already waiting on. This is a
 * ruled block that dims and lifts, and `motion-safe:` means it does not move at
 * all under `prefers-reduced-motion`.
 *
 * `aria-hidden`, because a skeleton has nothing to announce — the surface that
 * owns it announces the result through an Announcer when the data lands.
 */
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = withDefaults(defineProps<{ lines?: number; class?: HTMLAttributes["class"] }>(), {
  lines: 3,
});

/** Ragged widths read as text; identical bars read as a loading graphic. */
const WIDTHS = ["100%", "92%", "96%", "78%"];
const widthFor = (index: number): string =>
  index === props.lines ? "62%" : WIDTHS[(index - 1) % WIDTHS.length];
</script>

<template>
  <div :class="cn('space-y-2', props.class)" aria-hidden="true" data-testid="skeleton">
    <div
      v-for="line in lines"
      :key="line"
      data-testid="skeleton-line"
      class="h-4 border border-dashed border-border bg-muted motion-safe:animate-pulse"
      :style="{ width: widthFor(line) }"
    />
  </div>
</template>
