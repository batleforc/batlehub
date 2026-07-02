<script setup lang="ts">
import Dialog from "@/components/ui/dialog/Dialog.vue";
import { Button } from "@/components/ui/button";

withDefaults(
  defineProps<{
    open: boolean;
    title?: string;
    description?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    loadingLabel?: string;
    destructive?: boolean;
    loading?: boolean;
    error?: string | null;
  }>(),
  {
    confirmLabel: "Confirm",
    cancelLabel: "Cancel",
    destructive: false,
    loading: false,
    error: null,
  },
);

const emit = defineEmits<{
  "update:open": [boolean];
  confirm: [];
}>();

function cancel() {
  emit("update:open", false);
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <template #title
      ><slot name="title">{{ title }}</slot></template
    >
    <template v-if="description || $slots.description" #description>
      <slot name="description">{{ description }}</slot>
    </template>
    <div class="space-y-4">
      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
      <div class="flex justify-end gap-2">
        <Button variant="outline" size="sm" :disabled="loading" @click="cancel">
          {{ cancelLabel }}
        </Button>
        <Button
          :variant="destructive ? 'destructive' : 'default'"
          size="sm"
          :disabled="loading"
          @click="emit('confirm')"
        >
          {{ loading ? (loadingLabel ?? confirmLabel) : confirmLabel }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
