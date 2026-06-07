<script setup lang="ts">
import { TabsList, type TabsListProps } from "radix-vue";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsListProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsList
    v-bind="props"
    :class="
      cn(
        'inline-flex h-9 items-center justify-center rounded-sm bg-secondary p-0.5 text-muted-foreground border border-border/50',
        props.class,
      )
    "
  >
    <slot />
  </TabsList>
</template>
