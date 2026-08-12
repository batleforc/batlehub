<script setup lang="ts">
import { useI18n } from "vue-i18n";
/**
 * The operator's landing page, rebuilt around the two questions they actually
 * arrive with — *is it healthy* and *is it saving me anything* — rather than a
 * grid of four equal-weight counters where a degraded registry looked exactly
 * like a healthy one (RFC 0003 §6.4).
 *
 * The verdict leads. Numbers support it. A wall of stats is the shape you use
 * when you do not know which of them matters, and here we do: if a registry is
 * erroring, nothing else on this page is the story.
 */
import { computed } from "vue";
import { RouterLink } from "vue-router";
import { adminStats, registryHealth } from "@/client/sdk.gen";
import type { StatsResponse, RegistryHealthDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { formatBytes as fmtBytes, formatCount } from "@/lib/format";
import { PageHeader } from "@/components/ui/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/ui/empty-state";
import { Alert } from "@/components/ui/alert";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

const { t } = useI18n();

const { token } = useAuth();

const {
  data: stats,
  loading: statsLoading,
  error: statsError,
} = useApi<StatsResponse>(
  () => adminStats() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const {
  data: health,
  loading: healthLoading,
  error: healthError,
} = useApi<RegistryHealthDto[]>(
  () => registryHealth() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registries = computed<RegistryHealthDto[]>(() => health.value ?? []);
const degraded = computed(() => registries.value.filter((r) => (r.recent_errors?.length ?? 0) > 0));
const isFresh = computed(() => !healthLoading.value && registries.value.length === 0);

/** One sentence, and it is the first thing on the page. */
const verdict = computed(() => {
  if (healthLoading.value) return null;
  if (healthError.value) return { tone: "unknown" as const, text: "Health could not be read." };
  if (isFresh.value) return null;
  if (degraded.value.length === 0) {
    return {
      tone: "ok" as const,
      text: `All ${registries.value.length} registries are answering.`,
    };
  }
  const names = degraded.value.map((r) => r.registry).join(", ");
  return {
    tone: "bad" as const,
    text: `${degraded.value.length} of ${registries.value.length} registries reported errors: ${names}.`,
  };
});

/**
 * What the cache actually saved. `artifact_hits` is the count of downloads
 * served without going upstream; expressing it as a share of all downloads is
 * the number an operator can act on, where a raw hit count is not.
 */
const totalRequests = computed(
  () => (stats.value?.aggregate.artifact_hits ?? 0) + (stats.value?.aggregate.artifact_misses ?? 0),
);
const hitRate = computed(() => stats.value?.aggregate.hit_rate ?? null);
const fmtPct = (n: number | null): string => (n == null ? "—" : `${(n * 100).toFixed(1)}%`);
</script>

<template>
  <div class="space-y-6">
    <PageHeader title="Dashboard" />

    <Skeleton v-if="healthLoading" :lines="3" />

    <!-- The verdict, before any number. -->
    <div
      v-else-if="verdict"
      class="border px-4 py-3"
      :class="verdict.tone === 'bad' ? 'border-destructive' : 'border-border'"
    >
      <p class="font-mono text-sm" :class="verdict.tone === 'bad' ? 'text-destructive' : ''">
        {{ verdict.text }}
      </p>
      <p v-if="verdict.tone === 'bad'" class="mt-2">
        <Button as-child size="sm" variant="outline">
          <RouterLink to="/admin/observability/health">{{
            t("adminDashboard.openHealth")
          }}</RouterLink>
        </Button>
      </p>
    </div>

    <EmptyState
      v-if="isFresh"
      :title="t('adminDashboard.noRegistriesConfiguredYet')"
      description="Nothing is cached or served until a registry exists. Add a [[registries]] block to config.toml and reload."
    >
      <template #action>
        <Button as-child size="sm">
          <RouterLink to="/admin/operations/config-reload">{{
            t("adminDashboard.openConfig")
          }}</RouterLink>
        </Button>
      </template>
    </EmptyState>

    <template v-else>
      <Alert v-if="statsError" variant="destructive">{{ statsError }}</Alert>
      <Skeleton v-else-if="statsLoading" :lines="2" />

      <!-- What the cache saved, stated as a sentence rather than as four tiles. -->
      <p v-else-if="stats" class="text-sm text-muted-foreground">
        <i18n-t keypath="adminDashboard.cacheSummary" tag="span">
          <template #rate
            ><span class="font-mono text-foreground">{{ fmtPct(hitRate) }}</span></template
          >
          <template #total
            ><span class="font-mono text-foreground">{{
              formatCount(totalRequests)
            }}</span></template
          >
          <template #since
            ><span class="font-mono">{{
              new Date(stats.since_startup).toLocaleDateString()
            }}</span></template
          >
          <template #size
            ><span class="font-mono text-foreground">{{
              fmtBytes(stats.aggregate.cached_bytes)
            }}</span></template
          >
        </i18n-t>
      </p>

      <section v-if="stats && stats.per_registry.length > 0" class="space-y-2">
        <h2 class="font-mono text-xs uppercase tracking-wider text-muted-foreground">
          {{ t("adminDashboard.perRegistry") }}
        </h2>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Registry</TableHead>
              <TableHead class="text-right">{{ t("adminDashboard.hitRate") }}</TableHead>
              <TableHead class="text-right">Hits</TableHead>
              <TableHead class="text-right">Misses</TableHead>
              <TableHead class="text-right">Cached</TableHead>
              <TableHead>State</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="r in stats.per_registry" :key="r.registry">
              <TableCell class="font-mono text-sm">{{ r.registry }}</TableCell>
              <TableCell class="text-right font-mono text-sm">
                {{ fmtPct(r.hit_rate ?? null) }}
              </TableCell>
              <TableCell class="text-right font-mono text-sm text-muted-foreground">
                {{ formatCount(r.artifact_hits) }}
              </TableCell>
              <TableCell class="text-right font-mono text-sm text-muted-foreground">
                {{ formatCount(r.artifact_misses) }}
              </TableCell>
              <TableCell class="text-right font-mono text-sm text-muted-foreground">
                {{ fmtBytes(r.cached_bytes) }}
              </TableCell>
              <TableCell>
                <Badge
                  v-if="degraded.some((d) => d.registry === r.registry)"
                  variant="destructive"
                  class="text-xs"
                >
                  Errors
                </Badge>
                <span v-else class="text-xs text-muted-foreground">OK</span>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </section>
    </template>
  </div>
</template>
