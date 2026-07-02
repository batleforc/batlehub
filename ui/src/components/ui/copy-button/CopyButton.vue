<script setup lang="ts">
import { ref } from "vue";
import { Button } from "@/components/ui/button";

type ButtonVariant = "default" | "destructive" | "outline" | "secondary" | "ghost" | "link";
type ButtonSize = "default" | "sm" | "lg" | "icon";

const props = withDefaults(
  defineProps<{
    text: string;
    label?: string;
    copiedLabel?: string;
    size?: ButtonSize;
    variant?: ButtonVariant;
    resetMs?: number;
  }>(),
  {
    label: "Copy",
    copiedLabel: "Copied!",
    size: "sm",
    variant: "ghost",
    resetMs: 2000,
  },
);

const emit = defineEmits<{ copied: [] }>();

const copied = ref(false);

async function copy() {
  await navigator.clipboard.writeText(props.text);
  copied.value = true;
  emit("copied");
  setTimeout(() => {
    copied.value = false;
  }, props.resetMs);
}

defineOptions({ inheritAttrs: false });
</script>

<template>
  <Button :variant="variant" :size="size" v-bind="$attrs" @click="copy">
    <slot :copied="copied">{{ copied ? copiedLabel : label }}</slot>
  </Button>
</template>
