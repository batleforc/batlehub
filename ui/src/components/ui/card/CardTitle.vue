<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <h3 :class="cn('font-mono text-2xl font-bold leading-none tracking-tight', props.class)">
    <slot />
  </h3>
</template>
