<script setup lang="ts">
import Alert from "@/components/ui/alert/Alert.vue";

withDefaults(
  defineProps<{
    loading: boolean;
    error?: string | null;
    empty?: boolean;
    emptyMessage?: string;
  }>(),
  { error: null, empty: false, emptyMessage: "No results." },
);
</script>

<template>
  <template v-if="loading">
    <slot name="loading">
      <p class="text-sm text-muted-foreground py-4">Loading…</p>
    </slot>
  </template>
  <template v-else-if="error">
    <slot name="error" :error="error">
      <Alert variant="destructive">{{ error }}</Alert>
    </slot>
  </template>
  <template v-else-if="empty">
    <slot name="empty">
      <p class="text-sm text-muted-foreground text-center py-4">{{ emptyMessage }}</p>
    </slot>
  </template>
  <template v-else>
    <slot />
  </template>
</template>
