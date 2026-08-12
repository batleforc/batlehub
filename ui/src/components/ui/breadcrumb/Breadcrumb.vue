<script setup lang="ts">
/**
 * Trail navigation. The last entry is the current page: it is not a link, and
 * it carries `aria-current="page"` so assistive tech can say where you are.
 * The separators are decorative and hidden, or they get read out between every
 * pair of labels.
 */
import { RouterLink } from "vue-router";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";
import type { Crumb } from "./types";

const props = withDefaults(
  defineProps<{ items: Crumb[]; label?: string; class?: HTMLAttributes["class"] }>(),
  { label: "Breadcrumb" },
);
</script>

<template>
  <nav :aria-label="label" :class="cn('font-mono text-xs', props.class)">
    <ol class="flex flex-wrap items-center gap-1.5">
      <li
        v-for="(item, index) in items"
        :key="`${item.label}-${index}`"
        class="flex items-center gap-1.5"
      >
        <span v-if="index > 0" aria-hidden="true" class="text-border">/</span>
        <RouterLink
          v-if="item.to && index < items.length - 1"
          :to="item.to"
          class="text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
        >
          {{ item.label }}
        </RouterLink>
        <span
          v-else
          :aria-current="index === items.length - 1 ? 'page' : undefined"
          class="text-foreground"
        >
          {{ item.label }}
        </span>
      </li>
    </ol>
  </nav>
</template>
