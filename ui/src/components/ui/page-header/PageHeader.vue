<script setup lang="ts">
import { cn } from "@/lib/utils";

withDefaults(
  defineProps<{
    title?: string;
    description?: string;
    variant?: "default" | "display";
  }>(),
  { variant: "default" },
);
</script>

<template>
  <div class="flex items-start justify-between gap-4">
    <div>
      <h1
        :class="
          cn(
            'text-2xl font-semibold flex items-center gap-2',
            // The bitmap face at Pixel Medium — one per view, and the only step
            // above the data ramp. It carries no glow: depth in this world is
            // inked, not lit. Tracked 0.04em because Silkscreen's square pixel
            // sets tight at its own em.
            variant === 'display' && 'font-display font-bold tracking-[0.04em]',
          )
        "
      >
        <slot name="title">{{ title }}</slot>
      </h1>
      <p
        v-if="description || $slots.description"
        class="text-sm text-muted-foreground mt-0.5 max-w-[64ch]"
      >
        <slot name="description">{{ description }}</slot>
      </p>
    </div>
    <div v-if="$slots.actions" class="flex items-center gap-2 shrink-0">
      <slot name="actions" />
    </div>
  </div>
</template>
