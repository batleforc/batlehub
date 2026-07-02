<script setup lang="ts">
import { RouterLink, useRoute } from "vue-router";

defineProps<{
  tabs: { to: string; label: string }[];
}>();

const route = useRoute();

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <nav class="flex items-center gap-1 flex-wrap border-b border-border/60 mb-4">
    <RouterLink
      v-for="tab in tabs"
      :key="tab.to"
      :to="tab.to"
      :class="[
        'px-3 py-1.5 rounded-t-sm font-mono text-sm whitespace-nowrap transition-colors border-b-2 -mb-px',
        isActive(tab.to)
          ? 'border-primary text-foreground font-semibold'
          : 'border-transparent text-muted-foreground hover:text-accent-foreground',
      ]"
    >
      {{ tab.label }}
    </RouterLink>
  </nav>
</template>
