<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Alert } from "@/components/ui/alert";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{
    loading: boolean;
    error?: string | null;
    empty?: boolean;
    emptyMessage?: string;
  }>(),
  { error: null, empty: false, emptyMessage: undefined },
);

/* The default is resolved here rather than in `withDefaults`, which evaluates
   once at module scope and would freeze the English in place — including for a
   viewer who switches locale without a reload. */
const emptyText = computed(() => props.emptyMessage ?? t("asyncState.noResults"));
</script>

<template>
  <template v-if="loading">
    <slot name="loading">
      <p class="text-sm text-muted-foreground py-4">{{ t("asyncState.loading") }}</p>
    </slot>
  </template>
  <template v-else-if="error">
    <slot name="error" :error="error">
      <Alert variant="destructive">{{ error }}</Alert>
    </slot>
  </template>
  <template v-else-if="empty">
    <slot name="empty">
      <p class="text-sm text-muted-foreground text-center py-4">{{ emptyText }}</p>
    </slot>
  </template>
  <template v-else>
    <slot />
  </template>
</template>
