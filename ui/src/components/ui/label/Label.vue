<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  for?: string;
}>();
</script>

<template>
  <label
    :for="props.for"
    :class="
      cn(
        'font-mono text-xs font-semibold uppercase tracking-wide text-muted-foreground leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70',
        props.class,
      )
    "
  >
    <slot />
  </label>
</template>
