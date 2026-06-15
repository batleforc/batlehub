<script setup lang="ts">
import { type HTMLAttributes, computed } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  type?: string;
  placeholder?: string;
  disabled?: boolean;
  modelValue?: string | number;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const delegatedProps = computed(() => {
  const { class: _class, modelValue: _modelValue, ...rest } = props;
  return rest;
});
</script>

<template>
  <input
    v-bind="delegatedProps"
    :aria-label="placeholder"
    :value="modelValue"
    :class="
      cn(
        'flex h-9 w-full rounded-sm border border-input bg-background px-3 py-2 font-mono text-sm ring-offset-background placeholder:text-muted-foreground/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-0 disabled:cursor-not-allowed disabled:opacity-50 file:border-0 file:bg-transparent file:text-sm file:font-medium',
        props.class,
      )
    "
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
