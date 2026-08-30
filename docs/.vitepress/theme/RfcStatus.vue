<script setup lang="ts">
import { computed } from "vue";
import { useData } from "vitepress";

/**
 * The status banner every published RFC opens with (RFC 0005 §6.8).
 *
 * These documents are candid about defects that were live in shipped versions,
 * and three of the six are not implemented. A reader arriving from a search
 * result has no way to know whether they are reading history, a proposal, or a
 * description of the product today — and an `In review` RFC read as a shipped
 * one is a false claim about the product.
 *
 * The value is never written here. It is parsed out of the RFC's own `Status`
 * row by `transformPageData` in config.ts, because a second hand-maintained
 * copy of a fact drifts from the first, which is the defect RFC 0005 keeps
 * finding.
 */
const { frontmatter } = useData();

const status = computed(
  () =>
    frontmatter.value.rfcStatus as
      | { state: string; note: string; settled: boolean }
      | undefined,
);
</script>

<template>
  <aside v-if="status" class="rfc-status" :class="{ settled: status.settled }">
    <span class="state">{{ status.state }}</span>
    <span v-if="status.note" class="note">{{ status.note }}</span>
  </aside>
</template>

<style scoped>
/* Copper carries "proposed, not yet decided" — DESIGN.md gives it waiting and
   held, and it never means good. A settled RFC is a fact, so it takes ink and
   the strong rule instead. No fill separates the two: the rule and the word do
   it, and fill is not a dependable channel in either rendition. */
.rfc-status {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--s3);
  margin-bottom: var(--s5);
  padding: var(--s3) 0 var(--s3) var(--s4);
  border-left: 2px solid var(--copper);
}

.rfc-status.settled {
  border-left-color: var(--rule-strong);
}

.state {
  font-family: var(--face-display);
  font-weight: 700;
  font-size: var(--t-px-sm);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--copper);
}

.rfc-status.settled .state {
  color: var(--ink);
}

/* `overflow-wrap: anywhere` rather than nothing, because the note is the RFC's
   own prose and that prose names config keys. UAX #14 offers no break inside
   `[registries.grants]`/`[[registries.namespaces]]`, so it is one 470px run —
   wider than the 334px column at 390 — and a flex item's `min-width: auto` is
   its min-content width, so the banner sized itself to the token and pushed the
   document 40px past the viewport. `anywhere` is the value that also lowers the
   intrinsic minimum, which `break-word` does not, so the item shrinks to the
   column instead of merely spilling less far out of it. */
.note {
  font-size: var(--t-body);
  line-height: 1.6;
  color: var(--ink-dim);
  overflow-wrap: anywhere;
}
</style>
