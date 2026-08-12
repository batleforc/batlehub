<script setup lang="ts">
/**
 * Empty is two different states wearing one face, and conflating them is the
 * defect this component exists to prevent:
 *
 *   - nothing has ever been here  → say what would put something here
 *   - your filter matched nothing → say so, and offer to clear it
 *
 * A user who is told "no packages" while a filter is silently applied concludes
 * the registry is broken. `filtered` is therefore required rather than inferred.
 */
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    title: string;
    description?: string;
    filtered?: boolean;
    class?: HTMLAttributes["class"];
  }>(),
  { description: "", filtered: false },
);
</script>

<template>
  <div
    :class="cn('border border-border px-6 py-10 text-center', props.class)"
    data-testid="empty-state"
    :data-filtered="filtered ? 'true' : 'false'"
  >
    <p class="font-mono text-sm font-semibold text-foreground">{{ title }}</p>
    <p
      v-if="description || $slots.description"
      class="mx-auto mt-2 max-w-prose text-sm text-muted-foreground"
    >
      <slot name="description">{{ description }}</slot>
    </p>
    <div v-if="$slots.action" class="mt-4 flex justify-center gap-2">
      <slot name="action" />
    </div>
  </div>
</template>
