<script setup lang="ts">
import { type HTMLAttributes, computed } from "vue";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap font-mono font-semibold text-sm rounded-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-0 disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        default:
          "bg-primary text-primary-foreground [box-shadow:var(--cyber-glow)] hover:bg-primary/85 hover:-translate-y-px active:translate-y-0 transition-[box-shadow,background-color,transform]",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/85",
        outline:
          "border border-primary/40 bg-transparent text-primary hover:bg-accent hover:border-primary/70",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/70",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default: "h-9 px-4 py-2",
        sm: "h-8 px-3 text-xs",
        lg: "h-10 px-8",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

type ButtonVariants = VariantProps<typeof buttonVariants>;

const props = withDefaults(
  defineProps<{
    variant?: ButtonVariants["variant"];
    size?: ButtonVariants["size"];
    class?: HTMLAttributes["class"];
    disabled?: boolean;
  }>(),
  { variant: "default", size: "default" },
);

const delegatedProps = computed(() => {
  const { class: _class, variant: _variant, size: _size, ...rest } = props;
  return rest;
});
</script>

<template>
  <button v-bind="delegatedProps" :class="cn(buttonVariants({ variant, size }), props.class)">
    <slot />
  </button>
</template>
