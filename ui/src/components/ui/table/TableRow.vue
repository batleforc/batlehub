<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <tr
    :class="
      cn(
        'border-b border-border/60 transition-colors hover:bg-accent/50 data-[state=selected]:bg-accent',
        props.class,
      )
    "
  >
    <slot />
  </tr>
</template>
