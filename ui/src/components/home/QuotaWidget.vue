<script setup lang="ts">
/**
 * "Am I near my quota" (RFC 0004 §4.2), answered on `/` for the person it
 * concerns.
 *
 * Before this, the only read paths were `/api/v1/admin/quota*` — all admin-only
 * — so a developer could not see their own usage until a publish failed with a
 * 429. This calls `/api/v1/me/quota`, which takes no `user_id` at all.
 *
 * Two rendering rules the server has already applied, restated here because
 * they decide what this component *does not* draw:
 *
 *   - Registries with no quota are not in the response. Nothing to measure.
 *   - If none of the caller's registries has a quota, the response is empty and
 *     the widget does not render. An empty meter is worse than no meter: it
 *     invents a limit where the operator configured none.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { myQuota } from "@/client/sdk.gen";
import type { MyQuotaDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { formatBytes, formatCount } from "@/lib/format";
import { Alert } from "@/components/ui/alert";
import { Meter, type MeterState } from "@/components/ui/meter";
import { Skeleton } from "@/components/ui/skeleton";

const { t } = useI18n();
const { token } = useAuth();

const { data, error, loading } = useApi<MyQuotaDto[]>(
  () => myQuota() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const rows = computed<MyQuotaDto[]>(() => data.value ?? []);

type WireState = MyQuotaDto["state"];

/**
 * The server's verdict, not a threshold recomputed here. `warn_threshold_pct`
 * lives in config beside the limits that enforcement reads, and a second copy
 * of that rule in the browser is a copy that can disagree with the 429.
 *
 * Per *dimension*, not per row: a registry whose versions are at 82% while its
 * storage sits at 68% has crossed one threshold, not two, and colouring both
 * meters the same would say otherwise (RFC 0004 §4.2).
 */
function meterState(state: WireState | null | undefined): MeterState {
  if (state === "at_limit") return "at-limit";
  return state === "warning" ? "warning" : "ok";
}

/**
 * The caption is the state's *word*, which DESIGN.md requires alongside the
 * hue — state is never carried by hue alone. At the limit a publish is
 * genuinely refused (under `enforcement = "block"`); copper is a quota
 * approaching its limit, which is DESIGN.md's degradation job — moving the
 * wrong way, not yet refused. This widget and `AdminDashboard`'s trend both
 * assumed that job before the world had an entry for it; RFC 0004-bis §11/O1
 * added it to copper rather than growing the palette.
 *
 * It sits under the meter it describes, so which dimension it is about is
 * positional rather than something the sentence has to name.
 */
function caption(state: WireState | null | undefined, pct: number): string {
  if (state === "at_limit") return t("quotaWidget.atLimit");
  if (state === "warning") return t("quotaWidget.approaching", { pct });
  return "";
}

const bytesText = (row: MyQuotaDto) =>
  t("quotaWidget.ofLimit", {
    used: formatBytes(row.bytes_used),
    limit: formatBytes(row.bytes_limit ?? 0),
  });

const packagesText = (row: MyQuotaDto) =>
  t("quotaWidget.ofLimit", {
    used: formatCount(row.packages_used),
    limit: formatCount(row.packages_limit ?? 0),
  });
</script>

<template>
  <!-- No quota anywhere the caller can reach: render nothing at all. -->
  <section v-if="loading || error || rows.length" aria-labelledby="quota-widget-heading">
    <h2
      id="quota-widget-heading"
      class="font-mono text-xs uppercase tracking-wider text-muted-foreground"
    >
      {{ t("quotaWidget.title") }}
    </h2>

    <Skeleton v-if="loading" class="mt-3" :lines="2" />
    <Alert v-else-if="error" variant="destructive" class="mt-3">{{ error }}</Alert>

    <ul v-else class="mt-3 space-y-4">
      <li v-for="row in rows" :key="row.registry" class="border border-border p-4">
        <p class="font-mono text-sm font-semibold text-foreground">{{ row.registry }}</p>

        <div class="mt-3 space-y-3">
          <div v-if="row.bytes_limit != null">
            <Meter
              :value="row.bytes_used"
              :max="row.bytes_limit"
              :label="t('quotaWidget.storage')"
              :value-text="bytesText(row)"
              :state="meterState(row.bytes_state)"
            />
            <p
              v-if="caption(row.bytes_state, row.warn_threshold_pct)"
              class="mt-1 text-xs"
              :class="row.bytes_state === 'at_limit' ? 'text-destructive' : 'text-copper'"
              data-testid="quota-caption"
            >
              {{ caption(row.bytes_state, row.warn_threshold_pct) }}
            </p>
          </div>

          <div v-if="row.packages_limit != null">
            <Meter
              :value="row.packages_used"
              :max="row.packages_limit"
              :label="t('quotaWidget.versions')"
              :value-text="packagesText(row)"
              :state="meterState(row.packages_state)"
            />
            <p
              v-if="caption(row.packages_state, row.warn_threshold_pct)"
              class="mt-1 text-xs"
              :class="row.packages_state === 'at_limit' ? 'text-destructive' : 'text-copper'"
              data-testid="quota-caption"
            >
              {{ caption(row.packages_state, row.warn_threshold_pct) }}
            </p>
          </div>
        </div>
      </li>
    </ul>
  </section>
</template>
