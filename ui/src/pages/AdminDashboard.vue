<script setup lang="ts">
import { computed } from "vue";
import { adminStats, registryHealth } from "@/client/sdk.gen";
import type { StatsResponse, RegistryHealthDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

const { token } = useAuth();

const { data: stats, loading: statsLoading, error: statsError } = useApi<StatsResponse>(
  () => adminStats() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const { data: health, loading: healthLoading } = useApi<RegistryHealthDto[]>(
  () => registryHealth() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

function fmtBytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1_073_741_824) return (n / 1_073_741_824).toFixed(1) + " GiB";
  if (n >= 1_048_576) return (n / 1_048_576).toFixed(1) + " MiB";
  if (n >= 1_024) return (n / 1_024).toFixed(1) + " KiB";
  return n + " B";
}

function fmtPct(n: number | null | undefined): string {
  if (n == null) return "—";
  return (n * 100).toFixed(1) + "%";
}

const healthyCount = computed(() => health.value?.filter((h) => h.recent_errors?.length === 0).length ?? 0);
const totalCount = computed(() => health.value?.length ?? 0);
</script>

<template>
  <div class="space-y-6">
    <h1 class="text-lg font-semibold font-mono">Admin Dashboard</h1>

    <!-- Summary cards -->
    <div v-if="statsLoading" class="text-sm text-muted-foreground">Loading stats…</div>
    <div v-else-if="statsError" class="text-sm text-destructive">Failed to load stats.</div>
    <div v-else-if="stats" class="grid grid-cols-2 md:grid-cols-4 gap-4">
      <Card>
        <CardHeader class="pb-1">
          <CardTitle class="text-xs font-mono uppercase tracking-wider text-muted-foreground">
            Hit Rate
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p class="text-2xl font-semibold font-mono">{{ fmtPct(stats.aggregate.hit_rate) }}</p>
          <p class="text-xs text-muted-foreground mt-1">artifact cache</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-1">
          <CardTitle class="text-xs font-mono uppercase tracking-wider text-muted-foreground">
            Cache Hits
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p class="text-2xl font-semibold font-mono">{{ stats.aggregate.artifact_hits.toLocaleString() }}</p>
          <p class="text-xs text-muted-foreground mt-1">since {{ new Date(stats.since_startup).toLocaleDateString() }}</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-1">
          <CardTitle class="text-xs font-mono uppercase tracking-wider text-muted-foreground">
            Cache Misses
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p class="text-2xl font-semibold font-mono">{{ stats.aggregate.artifact_misses.toLocaleString() }}</p>
          <p class="text-xs text-muted-foreground mt-1">fetched from upstream</p>
        </CardContent>
      </Card>

      <Card>
        <CardHeader class="pb-1">
          <CardTitle class="text-xs font-mono uppercase tracking-wider text-muted-foreground">
            Cached Bytes
          </CardTitle>
        </CardHeader>
        <CardContent>
          <p class="text-2xl font-semibold font-mono">{{ fmtBytes(stats.aggregate.cached_bytes) }}</p>
          <p class="text-xs text-muted-foreground mt-1">stored in backend</p>
        </CardContent>
      </Card>
    </div>

    <!-- Health summary -->
    <div class="flex items-center gap-2 text-sm">
      <span v-if="healthLoading" class="text-muted-foreground">Checking health…</span>
      <template v-else>
        <Badge :variant="healthyCount === totalCount ? 'default' : 'destructive'">
          {{ healthyCount }} / {{ totalCount }} registries healthy
        </Badge>
        <a href="/admin/health" class="text-xs text-muted-foreground underline underline-offset-2">
          View details →
        </a>
      </template>
    </div>

    <!-- Per-registry stats table -->
    <Card v-if="stats && stats.per_registry.length > 0">
      <CardHeader class="pb-2">
        <CardTitle class="text-sm font-mono">Per-Registry Cache Stats</CardTitle>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Registry</TableHead>
              <TableHead class="text-right">Hit Rate</TableHead>
              <TableHead class="text-right">Hits</TableHead>
              <TableHead class="text-right">Misses</TableHead>
              <TableHead class="text-right">Cached</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="r in stats.per_registry" :key="r.registry">
              <TableCell class="font-mono text-xs">{{ r.registry }}</TableCell>
              <TableCell class="text-right font-mono text-xs">{{ fmtPct(r.hit_rate) }}</TableCell>
              <TableCell class="text-right font-mono text-xs">{{ r.artifact_hits.toLocaleString() }}</TableCell>
              <TableCell class="text-right font-mono text-xs">{{ r.artifact_misses.toLocaleString() }}</TableCell>
              <TableCell class="text-right font-mono text-xs">{{ fmtBytes(r.cached_bytes) }}</TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  </div>
</template>
