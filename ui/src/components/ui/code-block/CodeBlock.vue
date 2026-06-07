<script setup lang="ts">
import { ref, watch } from "vue";
import { useShiki } from "@/composables/useShiki";

const props = defineProps<{ code: string; lang: string }>();

const { highlight, ready } = useShiki();
const html = ref("");

function update() {
  const h = highlight(props.code, props.lang);
  if (h) html.value = h;
}

watch([() => props.code, () => props.lang, ready], update, { immediate: true });
</script>

<template>
  <div class="relative">
    <div v-if="html" class="shiki-wrapper" v-html="html" />
    <pre v-else class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto leading-relaxed">{{
      code
    }}</pre>
    <slot />
  </div>
</template>
