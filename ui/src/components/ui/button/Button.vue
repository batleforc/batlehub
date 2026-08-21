<script setup lang="ts">
import { type HTMLAttributes, computed } from "vue";
import { Primitive } from "radix-vue";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center whitespace-nowrap font-mono font-semibold text-sm rounded-sm transition-colors disabled:pointer-events-none disabled:opacity-50",
  {
    variants: {
      variant: {
        // The one primary action, and the only element in the system allowed a
        // box-shadow (DESIGN.md, Flat-At-Rest Rule). Flat at rest; on hover a
        // cut-out action ring rather than a halo; on :active the pixel step —
        // the button displaces sideways and leaves two stacked plates behind
        // it, like a mis-registered print pull. Both are zero-blur.
        default:
          "bg-primary text-primary-foreground hover:bg-primary/85 hover:[box-shadow:var(--action-ring)] active:[box-shadow:var(--pixel-step)] active:-translate-x-0.5 transition-[box-shadow,background-color,transform]",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/85",
        outline:
          "border border-primary/40 bg-transparent text-primary hover:bg-accent hover:border-primary/70",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/70",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        // Vertical padding is declared even though the fixed height plus
        // inline-flex centring already places the label: the height is what
        // *happens* to space it, the padding is what says it should be. Without
        // it the label measures flush to the fill edge, which is what a rendered
        // scan reports and what would actually happen if the height were dropped.
        default: "h-9 px-4 py-2",
        sm: "h-8 px-3 py-1.5 text-xs",
        lg: "h-10 px-8 py-2",
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
    /**
     * Render the slotted child as the root instead of wrapping it in a
     * `<button>`. Callers already passed `as-child` with a `RouterLink` inside;
     * until this existed it fell through as a plain attribute and produced
     * `<button><a>` — a nested interactive control, which axe flags and screen
     * readers announce unreliably.
     */
    asChild?: boolean;
    as?: string;
  }>(),
  { variant: "default", size: "default", as: "button" },
);

const delegatedProps = computed(() => {
  const {
    class: _class,
    variant: _variant,
    size: _size,
    asChild: _asChild,
    as: _as,
    ...rest
  } = props;
  return rest;
});
</script>

<template>
  <Primitive
    v-bind="delegatedProps"
    :as="as"
    :as-child="asChild"
    :class="cn(buttonVariants({ variant, size }), props.class)"
  >
    <slot />
  </Primitive>
</template>
