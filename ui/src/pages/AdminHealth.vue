<script setup lang="ts">
import { ref } from "vue";
import { registryHealth, adminStats, clearRegistryCache } from "@/client/sdk.gen";
import type { RegistryHealthDto, StatsResponse } from "@/client/types.gen";
import { useApi, extractMessage } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import {
  formatBytes as fmtBytes,
  formatDate as fmtDate,
  formatRelative as fmtRelative,
  formatCount,
} from "@/lib/format";
import { REGISTRY_TYPE_VARIANTS, variantFromMap } from "@/lib/badge-variants";
import { PageHeader } from "@/components/ui/page-header";
import { AsyncState } from "@/components/ui/async-state";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { OBSERVABILITY_TABS } from "@/config/adminSections";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import ConfirmDialog from "@/components/ConfirmDialog.vue";

const { token } = useAuth();

const { data, error, loading, reload } = useApi<RegistryHealthDto[]>(
  () => registryHealth() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const { data: statsData } = useApi<StatsResponse>(
  () => adminStats() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const expandedErrors = ref<Set<string>>(new Set());

const clearTarget = ref<string | null>(null);
const clearing = ref(false);
const clearError = ref<string | null>(null);

async function confirmClearCache() {
  if (!clearTarget.value) return;
  clearing.value = true;
  clearError.value = null;
  try {
    const { error: apiErr } = await clearRegistryCache({ path: { registry: clearTarget.value } });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    clearTarget.value = null;
    reload();
  } catch (e) {
    clearError.value = extractMessage(e);
  } finally {
    clearing.value = false;
  }
}

function toggleErrors(registry: string) {
  if (expandedErrors.value.has(registry)) {
    expandedErrors.value.delete(registry);
  } else {
    expandedErrors.value.add(registry);
  }
  expandedErrors.value = new Set(expandedErrors.value);
}

const ROLE_LABELS: Record<string, string> = {
  anonymous: "Anonymous",
  user: "Users",
  admin: "Admins",
};
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="OBSERVABILITY_TABS" />
    <PageHeader
      title="Registry Health"
      description="Live snapshot of each registry — packages, cache, pull rates, and recent errors."
      variant="glow"
    >
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="reload">
          {{ loading ? "Refreshing…" : "Refresh" }}
        </Button>
      </template>
    </PageHeader>

    <AsyncState
      :loading="loading && !data"
      :error="error"
      :empty="!!data && data.length === 0"
      empty-message="No registries configured."
    >
      <!-- Aggregate stats (since last restart) -->
      <Card v-if="statsData" class="border-muted/60">
        <CardHeader class="pb-2">
          <div class="flex items-center justify-between">
            <CardTitle class="text-sm font-medium text-muted-foreground uppercase tracking-wide">
              Cache performance since last restart
            </CardTitle>
            <span class="text-xs text-muted-foreground"
              >since {{ fmtRelative(statsData.since_startup) }}</span
            >
          </div>
        </CardHeader>
        <CardContent>
          <div class="grid grid-cols-2 sm:grid-cols-4 gap-3">
            <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Cache hit rate</p>
              <p
                class="text-2xl font-semibold tabular-nums"
                :class="
                  statsData.aggregate.hit_rate != null && statsData.aggregate.hit_rate >= 0.7
                    ? 'text-primary'
                    : statsData.aggregate.hit_rate != null && statsData.aggregate.hit_rate >= 0.4
                      ? 'text-copper'
                      : 'text-muted-foreground'
                "
              >
                {{
                  statsData.aggregate.hit_rate != null
                    ? `${(statsData.aggregate.hit_rate * 100).toFixed(1)}%`
                    : "—"
                }}
              </p>
              <p class="text-xs text-muted-foreground">artifact requests</p>
            </div>
            <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Cache hits</p>
              <p class="text-2xl font-semibold tabular-nums text-primary">
                {{ formatCount(statsData.aggregate.artifact_hits) }}
              </p>
              <p class="text-xs text-muted-foreground">served from cache</p>
            </div>
            <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Cache misses</p>
              <p class="text-2xl font-semibold tabular-nums">
                {{ formatCount(statsData.aggregate.artifact_misses) }}
              </p>
              <p class="text-xs text-muted-foreground">fetched from upstream</p>
            </div>
            <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Total cached</p>
              <p class="text-2xl font-semibold">
                {{ fmtBytes(statsData.aggregate.cached_bytes) }}
              </p>
              <p class="text-xs text-muted-foreground">in storage</p>
            </div>
          </div>
        </CardContent>
      </Card>

      <!-- Registry cards grid -->
      <div
        v-if="data && data.length > 0"
        class="grid gap-4 sm:grid-cols-1 lg:grid-cols-2 xl:grid-cols-2"
      >
        <Card v-for="reg in data" :key="reg.registry" class="flex flex-col">
          <CardHeader class="pb-2">
            <div class="flex items-center justify-between gap-2">
              <CardTitle class="text-base font-mono">
                {{ reg.registry }}
              </CardTitle>
              <div class="flex items-center gap-2 shrink-0">
                <Badge
                  :variant="variantFromMap(reg.registry_type, REGISTRY_TYPE_VARIANTS)"
                  class="text-xs uppercase"
                >
                  {{ reg.registry_type }}
                </Badge>
                <Button
                  variant="outline"
                  size="sm"
                  class="text-xs h-6 px-2"
                  @click="clearTarget = reg.registry"
                >
                  Clear Cache
                </Button>
              </div>
            </div>
          </CardHeader>

          <CardContent class="flex-1 space-y-4">
            <!-- Stats row -->
            <div class="grid grid-cols-2 sm:grid-cols-3 gap-3">
              <!-- Packages -->
              <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
                <p class="text-xs text-muted-foreground">Packages</p>
                <p class="text-xl font-semibold tabular-nums">
                  {{ formatCount(reg.package_count) }}
                </p>
                <p class="text-xs text-muted-foreground">tracked</p>
              </div>

              <!-- Cache size -->
              <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
                <p class="text-xs text-muted-foreground">Cache size</p>
                <p class="text-xl font-semibold">
                  {{ fmtBytes(reg.total_size_bytes ?? null) }}
                </p>
                <p class="text-xs text-muted-foreground">
                  {{ reg.cached_artifact_count }} artifacts
                </p>
              </div>

              <!-- Last pull -->
              <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
                <p class="text-xs text-muted-foreground">Last pull</p>
                <p class="text-base font-semibold">
                  {{ fmtRelative(reg.last_pull_at ?? null) }}
                </p>
                <p v-if="reg.last_pull_at" class="text-xs text-muted-foreground">
                  {{ fmtDate(reg.last_pull_at ?? "") }}
                </p>
              </div>

              <!-- Pulls / hour -->
              <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
                <p class="text-xs text-muted-foreground">Pulls / hour</p>
                <p
                  class="text-xl font-semibold tabular-nums"
                  :class="reg.pulls_last_hour > 0 ? 'text-primary' : 'text-muted-foreground'"
                >
                  {{ formatCount(reg.pulls_last_hour) }}
                </p>
              </div>

              <!-- Pulls / day -->
              <div class="rounded-sm border bg-muted/30 p-3 space-y-0.5">
                <p class="text-xs text-muted-foreground">Pulls / day</p>
                <p class="text-xl font-semibold tabular-nums">
                  {{ formatCount(reg.pulls_last_day) }}
                </p>
              </div>
            </div>

            <!-- Recent errors -->
            <div>
              <button
                class="flex items-center gap-2 w-full text-left font-mono text-sm font-medium py-1 hover:text-accent-foreground transition-colors"
                @click="toggleErrors(reg.registry)"
              >
                <span
                  v-if="reg.recent_errors.length === 0"
                  class="flex items-center gap-1.5 text-green-600 dark:text-green-400"
                >
                  <span class="relative flex h-2 w-2 shrink-0">
                    <span
                      class="animate-ping absolute inline-flex h-full w-full rounded-sm bg-green-500 opacity-75"
                    />
                    <span class="relative inline-flex h-2 w-2 rounded-sm bg-green-500" />
                  </span>
                  No errors in the last 24 h
                </span>
                <span v-else class="flex items-center gap-1.5 text-destructive">
                  <span class="inline-block h-2 w-2 rounded-sm bg-destructive" />
                  {{ reg.recent_errors.length }} error{{ reg.recent_errors.length > 1 ? "s" : "" }}
                  in 24 h
                  <span class="text-muted-foreground text-xs ml-auto">
                    {{ expandedErrors.has(reg.registry) ? "▲ hide" : "▼ show" }}
                  </span>
                </span>
              </button>

              <div
                v-if="expandedErrors.has(reg.registry) && reg.recent_errors.length > 0"
                class="mt-2 rounded-sm border overflow-x-auto"
              >
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead class="text-xs"> When </TableHead>
                      <TableHead class="text-xs"> User </TableHead>
                      <TableHead class="text-xs"> Package </TableHead>
                      <TableHead class="text-xs"> Type </TableHead>
                      <TableHead class="text-xs"> Reason </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    <TableRow
                      v-for="err in reg.recent_errors"
                      :key="err.timestamp + err.package_name"
                    >
                      <TableCell class="text-xs whitespace-nowrap">
                        {{ fmtRelative(err.timestamp) }}
                      </TableCell>
                      <TableCell class="text-xs">
                        <span v-if="err.user_id">{{ err.user_id }}</span>
                        <span v-else class="text-muted-foreground italic">anonymous</span>
                      </TableCell>
                      <TableCell class="font-mono text-xs">
                        {{ err.package_name
                        }}<span class="text-muted-foreground">@{{ err.version }}</span>
                      </TableCell>
                      <TableCell>
                        <Badge
                          :variant="err.error_type === 'error' ? 'destructive' : 'secondary'"
                          class="text-xs"
                        >
                          {{ err.error_type === "error" ? "Upstream error" : "Denied" }}
                        </Badge>
                      </TableCell>
                      <TableCell
                        class="text-xs text-muted-foreground max-w-[200px] truncate"
                        :title="err.reason"
                      >
                        {{ err.reason }}
                      </TableCell>
                    </TableRow>
                  </TableBody>
                </Table>
              </div>
            </div>

            <!-- Who has access -->
            <div class="space-y-1.5">
              <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide">
                Who has access
              </p>
              <div class="flex flex-wrap gap-1.5">
                <Badge
                  v-for="role in reg.access.roles"
                  :key="role"
                  variant="secondary"
                  class="text-xs"
                >
                  {{ ROLE_LABELS[role] ?? role }}
                </Badge>
                <Badge
                  v-for="group in reg.access.groups"
                  :key="group"
                  variant="outline"
                  class="text-xs font-mono"
                >
                  {{ group }}
                </Badge>
                <Badge
                  v-if="reg.access.roles.length === 0 && reg.access.groups.length === 0"
                  variant="destructive"
                  class="text-xs"
                >
                  No access configured
                </Badge>
                <span
                  v-else-if="
                    !reg.access.roles.includes('anonymous') && !reg.access.roles.includes('user')
                  "
                  class="text-xs text-copper flex items-center gap-1"
                >
                  ⚠ Restricted — no public access
                </span>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>
    </AsyncState>
  </div>

  <!-- Clear cache confirmation dialog -->
  <ConfirmDialog
    :open="clearTarget !== null"
    confirm-label="Clear Cache"
    loading-label="Clearing…"
    destructive
    :loading="clearing"
    :error="clearError"
    @update:open="
      (v) => {
        if (!v) {
          clearTarget = null;
          clearError = null;
        }
      }
    "
    @confirm="confirmClearCache"
  >
    <template #title>
      Clear cache for <span class="font-mono">{{ clearTarget }}</span
      >?
    </template>
    <template #description>
      All cached artifacts for this registry will be permanently removed. Packages will be
      re-fetched from upstream on the next request.
    </template>
  </ConfirmDialog>
</template>
