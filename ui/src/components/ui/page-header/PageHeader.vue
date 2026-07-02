<script setup lang="ts">
import { cn } from "@/lib/utils";

withDefaults(
  defineProps<{
    title?: string;
    description?: string;
    variant?: "default" | "glow";
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
            variant === 'glow' && 'font-mono font-bold cyber-text-glow',
          )
        "
      >
        <slot name="title">{{ title }}</slot>
      </h1>
      <p v-if="description || $slots.description" class="text-sm text-muted-foreground mt-0.5">
        <slot name="description">{{ description }}</slot>
      </p>
    </div>
    <div v-if="$slots.actions" class="flex items-center gap-2 shrink-0">
      <slot name="actions" />
    </div>
  </div>
</template>
