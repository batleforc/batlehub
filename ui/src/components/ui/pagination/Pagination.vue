<script setup lang="ts">
import { computed } from "vue";
import { Button } from "@/components/ui/button";

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
  <div class="flex items-center justify-between">
    <Button variant="outline" size="sm" :disabled="disabled || !hasPrev" @click="prev">
      Previous
    </Button>
    <span class="text-xs text-muted-foreground">
      Page {{ page + 1 }}<template v-if="totalPages !== undefined"> of {{ totalPages }}</template>
    </span>
    <Button variant="outline" size="sm" :disabled="disabled || !canGoNext" @click="next">
      Next
    </Button>
  </div>
</template>
