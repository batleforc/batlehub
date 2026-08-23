<script setup lang="ts">
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { Info, CloudOff, Clock } from "@lucide/vue";
import type { UpstreamReadDto } from "@/client/types.gen";

/**
 * What the discovery read did, in one sentence.
 *
 * A component rather than three inline `v-if`s on the page, so the states of
 * RFC 0007 §4.4's rung list have one implementation that cannot disagree with
 * itself: *answered from cache*, *fetched now*, *served stale and how old*, and
 * *upstream unreachable*.
 *
 * Silent when the read was not attempted, which is the common case — a
 * `local`-mode registry, a package published here, or a registry with the read
 * off. Nothing happened, so there is nothing to say; a banner reading "we did
 * not ask upstream" on every internal package would be noise.
 */
const props = defineProps<{
  upstream: UpstreamReadDto;
  /** Upstream-only rows currently on the page, for the count in the notice. */
  upstreamVersionCount: number;
}>();

const { t } = useI18n();

type NoticeKind = "unreachable" | "stale" | "added" | null;

const kind = computed<NoticeKind>(() => {
  if (!props.upstream.attempted) return null;
  if (props.upstream.error) return "unreachable";
  if (props.upstream.freshness === "stale") return "stale";
  // Nothing new to report when the upstream knew nothing this instance did not
  // already hold: the table is complete either way.
  return props.upstreamVersionCount > 0 ? "added" : null;
});

const icon = computed(() => {
  switch (kind.value) {
    case "unreachable":
      return CloudOff;
    case "stale":
      return Clock;
    default:
      return Info;
  }
});

const message = computed(() => {
  switch (kind.value) {
    case "unreachable":
      return t("upstreamNotice.unreachable");
    case "stale":
      return t("upstreamNotice.stale");
    case "added":
      return props.upstream.truncated
        ? t("upstreamNotice.addedTruncated", {
            count: props.upstreamVersionCount,
          })
        : t("upstreamNotice.added", { count: props.upstreamVersionCount });
    default:
      return null;
  }
});
</script>

<template>
  <output
    v-if="message"
    class="flex items-start gap-2 text-sm text-muted-foreground"
    :class="kind === 'unreachable' ? 'text-destructive' : ''"
  >
    <component :is="icon" class="mt-1 h-4 w-4 shrink-0" />
    <span>{{ message }}</span>
  </output>
</template>
