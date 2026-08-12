<script setup lang="ts">
/**
 * A bounded quantity against its limit (RFC 0004 §4.2).
 *
 * The useful fact about a quota is the *distance to the limit*, not the number
 * — "820 B used" says nothing without "of 1 kB" beside it. That is what a meter
 * is for, and why this is a primitive rather than a div with a width.
 *
 * ## The accessibility contract
 *
 * A bar is not a value. `role="meter"` is a gauge, not a progress indicator: it
 * reports a measurement that can move in both directions, which is what quota
 * usage does. Screen readers announce it from three things, all required here:
 *
 *   - an accessible **name** — the visible label, wired by `aria-labelledby`
 *     rather than duplicated into an `aria-label` that can drift from it
 *   - `aria-valuenow` / `aria-valuemin` / `aria-valuemax` — the raw numbers
 *   - `aria-valuetext` — the same sentence a sighted reader gets, because
 *     "aria-valuenow: 820" is not the fact; "820 B of 1.0 KiB" is
 *
 * ## Why the fill is not the state channel
 *
 * DESIGN.md's Undependable Fill Rule says a fill may never be the *only* signal
 * for a state, and the world never carries state by hue alone — pattern, word
 * and hue all say it. So the state here is carried by the value text (word) and
 * the caption the caller renders, with hue confirming: ink while ordinary,
 * `--copper` for "waiting rather than refused", crimson only once the limit is
 * reached, because at that point a publish genuinely *is* refused.
 *
 * The fill encodes a quantity, which is a different job from separating two
 * surfaces — but it is still not load-bearing on its own: remove all colour and
 * the meter still reads, because the numbers are written out.
 */
import { computed, useId, type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

export type MeterState = "ok" | "warning" | "at-limit";

const props = withDefaults(
  defineProps<{
    /** Current usage, in the same unit as `max`. */
    value: number;
    /** The limit. A `max` of 0 or less has no meaning for a meter and renders empty. */
    max: number;
    /** Visible label, and the meter's accessible name. */
    label: string;
    /** Human-readable value, e.g. `"820 B of 1.0 KiB"`. Shown, and announced. */
    valueText: string;
    state?: MeterState;
    class?: HTMLAttributes["class"];
  }>(),
  { state: "ok" },
);

const labelId = `meter-label-${useId()}`;

/**
 * Clamped on both ends. Usage past the limit is possible — `enforcement =
 * "warn"` records the publish and logs it — and a bar wider than its track
 * would overflow rather than communicate; `at-limit` is what says it.
 */
const percent = computed(() => {
  if (!Number.isFinite(props.max) || props.max <= 0) return 0;
  return Math.min(100, Math.max(0, (props.value / props.max) * 100));
});

const fillClass = computed(
  () =>
    ({
      ok: "bg-foreground",
      warning: "bg-copper",
      "at-limit": "bg-destructive",
    })[props.state],
);
</script>

<template>
  <div :class="cn('space-y-1', props.class)" data-testid="meter">
    <div class="flex items-baseline justify-between gap-2">
      <span :id="labelId" class="font-mono text-xs text-muted-foreground">{{ label }}</span>
      <span class="font-mono text-xs tabular-nums text-foreground">{{ valueText }}</span>
    </div>
    <div
      role="meter"
      :aria-labelledby="labelId"
      :aria-valuenow="value"
      :aria-valuemin="0"
      :aria-valuemax="max"
      :aria-valuetext="valueText"
      :data-state="state"
      class="h-2 w-full border border-border"
    >
      <!-- Presentational: the value it depicts is already on the `meter` above,
           and a second announcement of the same number is noise. -->
      <div
        aria-hidden="true"
        :class="cn('h-full', fillClass)"
        :style="{ width: `${percent}%` }"
      ></div>
    </div>
  </div>
</template>
