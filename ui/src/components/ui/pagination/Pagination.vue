<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { computed } from "vue";
import { Button } from "@/components/ui/button";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{
    /** 0-indexed current page. */
    page: number;
    totalPages?: number;
    /** Used instead of `totalPages` when the total page count isn't known upfront. */
    hasNext?: boolean;
    disabled?: boolean;
  }>(),
  { disabled: false },
);

const emit = defineEmits<{ "update:page": [number] }>();

const hasPrev = computed(() => props.page > 0);
const canGoNext = computed(() =>
  props.totalPages !== undefined ? props.page < props.totalPages - 1 : (props.hasNext ?? true),
);

function prev() {
  if (hasPrev.value) emit("update:page", props.page - 1);
}
function next() {
  if (canGoNext.value) emit("update:page", props.page + 1);
}
</script>

<template>
  <!-- A navigation landmark, so it is reachable by landmark rather than only by
       tabbing to it; the indicator is a live region because paging changes the
       table underneath it without moving focus. -->
  <nav class="flex items-center justify-between" :aria-label="t('pagination.label')">
    <Button variant="outline" size="sm" :disabled="disabled || !hasPrev" @click="prev">
      {{ t("common.previous") }}
    </Button>
    <!-- Two whole messages rather than a sentence assembled around a value:
         French does not put "sur" where English puts "of", and the count is not
         always known. -->
    <output class="text-xs text-muted-foreground" aria-live="polite">
      {{
        totalPages === undefined
          ? t("pagination.page", { page: page + 1 })
          : t("pagination.pageOf", { page: page + 1, total: totalPages })
      }}
    </output>
    <Button variant="outline" size="sm" :disabled="disabled || !canGoNext" @click="next">
      {{ t("common.next") }}
    </Button>
  </nav>
</template>
