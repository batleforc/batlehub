<script setup lang="ts">
/**
 * A polite live region, so an async result is announced rather than only shown.
 *
 * The craft floor requires it and PRODUCT.md makes WCAG 2.2 AA binding: a table
 * that silently swaps its rows tells a screen-reader user nothing happened.
 * `assertive` is for errors that interrupt a task, never for routine success —
 * an assertive region cuts off whatever the user is currently hearing.
 */
withDefaults(defineProps<{ message?: string; assertive?: boolean }>(), {
  message: "",
  assertive: false,
});
</script>

<template>
  <p
    class="sr-only"
    :role="assertive ? 'alert' : 'status'"
    :aria-live="assertive ? 'assertive' : 'polite'"
    aria-atomic="true"
  >
    {{ message }}
  </p>
</template>
