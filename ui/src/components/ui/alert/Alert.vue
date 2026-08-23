<script setup lang="ts">
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const alertVariants = cva(
  "relative w-full rounded-lg border p-4 [&>svg~*]:pl-7 [&>svg+div]:translate-y-[-3px] [&>svg]:absolute [&>svg]:left-4 [&>svg]:top-4 [&>svg]:text-foreground",
  {
    variants: {
      variant: {
        default: "bg-background text-foreground",
        /* Opaque. At `/50` this measured 2.62:1 in light against a 3:1 floor
           — and `dark:border-destructive` already made it opaque in dark, so
           the two renditions disagreed about how loud a refusal is. One value
           now, and the `dark:` override has nothing left to override. */
        destructive: "border-destructive text-destructive [&>svg]:text-destructive",
        // "known" in the One Synthetic Rule's terms: a confirmation is full ink
        // against a rule that carries more weight, not a fifth hue. Green was
        // outside this palette entirely and its pairing was never measured.
        success: "border-foreground/50 bg-background text-foreground [&>svg]:text-foreground",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

type AlertVariants = VariantProps<typeof alertVariants>;

const props = withDefaults(
  defineProps<{
    variant?: AlertVariants["variant"];
    class?: string;
  }>(),
  { variant: "default" },
);
</script>

<template>
  <div :class="cn(alertVariants({ variant }), props.class)" role="alert">
    <slot />
  </div>
</template>
