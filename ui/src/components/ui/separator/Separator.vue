<script setup lang="ts">
import { Separator as SeparatorRoot } from "radix-vue";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    orientation?: "horizontal" | "vertical";
    class?: string;
  }>(),
  { orientation: "horizontal" }
);
</script>

<template>
  <SeparatorRoot
    :orientation="orientation"
    :class="cn(
      'shrink-0 bg-border',
      orientation === 'horizontal' ? 'h-[1px] w-full' : 'h-full w-[1px]',
      props.class
    )"
  />
</template>
