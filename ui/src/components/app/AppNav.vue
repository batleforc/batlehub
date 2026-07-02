<script setup lang="ts">
import { RouterLink, useRoute } from "vue-router";

defineProps<{
  links: { to: string; label: string }[];
  variant: "desktop" | "mobile";
}>();

const emit = defineEmits<{ navigate: [] }>();

const route = useRoute();

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <nav v-if="variant === 'desktop'" class="hidden md:flex items-center gap-0.5 text-sm">
    <RouterLink
      v-for="link in links"
      :key="link.to"
      :to="link.to"
      :class="[
        'px-3 py-1.5 rounded-sm font-mono text-sm transition-colors',
        isActive(link.to)
          ? 'bg-accent text-accent-foreground font-semibold'
          : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
      ]"
    >
      {{ link.label }}
    </RouterLink>
  </nav>
  <template v-else>
    <RouterLink
      v-for="link in links"
      :key="link.to"
      :to="link.to"
      :class="[
        'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
        isActive(link.to)
          ? 'bg-accent text-accent-foreground font-semibold'
          : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
      ]"
      @click="emit('navigate')"
    >
      {{ link.label }}
    </RouterLink>
  </template>
</template>
