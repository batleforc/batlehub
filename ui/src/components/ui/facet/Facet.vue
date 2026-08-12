<script setup lang="ts">
/**
 * A single-select filter list — the registry sidebar, extracted from
 * PackageExplorer where it was inlined.
 *
 * Selection is carried by ink and a 1px lit edge, not by a fill. That is not a
 * style preference: at this palette's lightness a fill step measures ~1.06:1
 * against the ground and ~1.11:1 on paper, so it is invisible to a great many
 * people while looking fine to the author (see The Undependable Fill Rule in
 * DESIGN.md). The fill stays as a secondary cue only.
 */
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";
import type { FacetOption } from "./types";

const props = withDefaults(
  defineProps<{
    /** `null` means "all" — rendered as its own option when `allLabel` is set. */
    modelValue: string | null;
    options: FacetOption[];
    label: string;
    allLabel?: string;
    class?: HTMLAttributes["class"];
  }>(),
  { allLabel: "" },
);

const emit = defineEmits<{ "update:modelValue": [string | null] }>();

const isActive = (value: string | null): boolean => props.modelValue === value;

/** Counts are numbers, so they get the reader's locale rather than raw digits. */
const formatCount = (count: number): string => new Intl.NumberFormat().format(count);
</script>

<template>
  <div :class="cn('font-mono', props.class)">
    <p
      :id="`facet-${label}`"
      class="px-3 pb-2 text-xs uppercase tracking-wider text-muted-foreground"
    >
      {{ label }}
    </p>
    <ul class="list-none p-0 m-0" :aria-labelledby="`facet-${label}`">
      <li v-if="allLabel">
        <button
          type="button"
          class="flex w-full items-baseline gap-2 border-l px-3 py-1.5 text-left text-sm transition-colors"
          :class="
            isActive(null)
              ? 'border-l-primary bg-secondary text-foreground font-semibold'
              : 'border-l-transparent text-muted-foreground hover:text-foreground'
          "
          :aria-pressed="isActive(null)"
          @click="emit('update:modelValue', null)"
        >
          {{ allLabel }}
        </button>
      </li>
      <li v-for="option in options" :key="option.value">
        <button
          type="button"
          class="flex w-full items-baseline gap-2 border-l px-3 py-1.5 text-left text-sm transition-colors"
          :class="
            isActive(option.value)
              ? 'border-l-primary bg-secondary text-foreground font-semibold'
              : 'border-l-transparent text-muted-foreground hover:text-foreground'
          "
          :aria-pressed="isActive(option.value)"
          @click="emit('update:modelValue', option.value)"
        >
          <span class="truncate">{{ option.label }}</span>
          <span
            v-if="option.count !== undefined"
            class="ml-auto text-xs tabular-nums text-muted-foreground"
          >
            {{ formatCount(option.count) }}
          </span>
        </button>
      </li>
    </ul>
  </div>
</template>
