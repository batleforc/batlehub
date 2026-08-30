<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { useI18n } from "vue-i18n";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"]; label?: string }>();

const { t } = useI18n();
</script>

<template>
  <!--
    The scroll container is focusable, and has to be. A region that scrolls but
    cannot be reached by keyboard is unreachable content for anyone not using a
    pointer — axe reports it as `scrollable-region-focusable`, and RFC 0003
    §4.7 makes tables keyboard-operable explicitly. `tabindex="0"` puts it in
    the tab order so arrow keys can scroll it; `role="region"` plus a name is
    what makes it announce as something worth stopping on rather than as an
    anonymous focus stop.

    This is also the Own-Container Overflow Rule's container: wide content
    scrolls here so the body never scrolls sideways.
  -->
  <!-- `<section>`, not `<div role="region">`: a named section carries the role
       natively, and the native element is the one assistive technology has the
       fewest ways to get wrong. -->
  <section
    class="relative w-full overflow-auto"
    tabindex="0"
    :aria-label="label ?? t('a11y.tableScroll')"
  >
    <table :class="cn('w-full caption-bottom text-sm', props.class)">
      <slot />
    </table>
  </section>
</template>
