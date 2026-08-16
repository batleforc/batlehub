<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-sm border px-2 py-0.5 text-xs font-mono font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      // No alpha fill under the accent text. The `bg-…/10` utilities these drop
      // put each colour on a 10% tint of itself, which nothing had ever
      // measured: on paper that lands `#c50220` on `#ecd0d3` — 4.26:1, under the
      // AA floor, at 12px. It is R10's finding again in a different place, and
      // the Undependable Fill Rule's point exactly: a fill is not a state
      // channel. The border carries the state, and the text sits on the card
      // ground, where it measures 5.6:1.
      variant: {
        default: "border-primary/40 text-primary",
        secondary: "border-secondary bg-secondary text-secondary-foreground",
        destructive: "border-destructive/40 text-destructive",
        outline: "border-border text-muted-foreground",
        copper: "border-copper/40 text-copper",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

type BadgeVariants = VariantProps<typeof badgeVariants>;

const props = withDefaults(
  defineProps<{
    variant?: BadgeVariants["variant"];
    class?: HTMLAttributes["class"];
  }>(),
  { variant: "default" },
);
</script>

<template>
  <div :class="cn(badgeVariants({ variant }), props.class)">
    <slot />
  </div>
</template>
