<script setup lang="ts">
import { TabsRoot, type TabsRootProps } from "radix-vue";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsRootProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsRoot
    v-bind="props"
    :class="cn('', props.class)"
  >
    <slot />
  </TabsRoot>
</template>
