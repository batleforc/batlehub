<script setup lang="ts">
import { ref } from "vue";
import { useAuth } from "@/composables/useAuth";
import { useApi } from "@/composables/useApi";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";
import Dialog from "@/components/ui/dialog/Dialog.vue";

const { token } = useAuth();

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";

interface RegistryAccessInfo {
  roles: string[];
  groups: string[];
}

interface RecentErrorDto {
  timestamp: string;
  user_id: string | null;
  package_name: string;
  version: string;
  error_type: string;
  reason: string;
}

interface RegistryHealthDto {
  registry: string;
  registry_type: string;
  package_count: number;
  cached_artifact_count: number;
  total_size_bytes: number | null;
  last_pull_at: string | null;
  pulls_last_hour: number;
  pulls_last_day: number;
  recent_errors: RecentErrorDto[];
  access: RegistryAccessInfo;
}

const { data, error, loading, reload } = useApi<RegistryHealthDto[]>(
  () =>
    fetch(`${API_BASE}/api/v1/admin/health`, {
      headers: token.value ? { Authorization: `Bearer ${token.value}` } : {},
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>,
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
    const r = await fetch(
      `${API_BASE}/api/v1/admin/registries/${encodeURIComponent(clearTarget.value)}/clear-cache`,
      {
        method: "POST",
        headers: token.value ? { Authorization: `Bearer ${token.value}` } : {},
      },
    );
    if (!r.ok) throw new Error(await r.text());
    clearTarget.value = null;
    reload();
  } catch (e) {
    clearError.value = e instanceof Error ? e.message : "Unknown error";
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

function fmtBytes(bytes: number | null): string {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(1)} GB`;
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  if (bytes >= 1_024) return `${(bytes / 1_024).toFixed(0)} KB`;
  return `${bytes} B`;
}

function fmtRelative(iso: string | null): string {
  if (!iso) return "Never";
  const diff = Date.now() - new Date(iso).getTime();
  const minutes = Math.floor(diff / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleString();
}

const ROLE_LABELS: Record<string, string> = {
  anonymous: "Anonymous",
  user: "Users",
  admin: "Admins",
};

const REGISTRY_TYPE_VARIANTS: Record<string, string> = {
  npm: "default",
  cargo: "secondary",
  github: "outline",
  openvsx: "secondary",
  goproxy: "outline",
};
</script>

<template>
  <div class="space-y-6">
    <!-- Page header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">Registry Health</h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Live snapshot of each registry — packages, cache, pull rates, and recent errors.
        </p>
      </div>
      <Button variant="outline" size="sm" :disabled="loading" @click="reload">
        {{ loading ? "Refreshing…" : "Refresh" }}
      </Button>
    </div>

    <p v-if="loading && !data" class="text-sm text-muted-foreground">Loading…</p>
    <p v-else-if="error" class="text-sm text-destructive">{{ error }}</p>
    <p v-else-if="data && data.length === 0" class="text-sm text-muted-foreground">No registries configured.</p>

    <!-- Registry cards grid -->
    <div
      v-if="data && data.length > 0"
      class="grid gap-4 sm:grid-cols-1 lg:grid-cols-2 xl:grid-cols-2"
    >
      <Card v-for="reg in data" :key="reg.registry" class="flex flex-col">
        <CardHeader class="pb-2">
          <div class="flex items-center justify-between gap-2">
            <CardTitle class="text-base font-mono">{{ reg.registry }}</CardTitle>
            <div class="flex items-center gap-2 shrink-0">
              <Badge
                :variant="(REGISTRY_TYPE_VARIANTS[reg.registry_type] as any) ?? 'outline'"
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
            <div class="rounded-lg border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Packages</p>
              <p class="text-xl font-semibold tabular-nums">{{ reg.package_count.toLocaleString() }}</p>
              <p class="text-xs text-muted-foreground">tracked</p>
            </div>

            <!-- Cache size -->
            <div class="rounded-lg border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Cache size</p>
              <p class="text-xl font-semibold">{{ fmtBytes(reg.total_size_bytes) }}</p>
              <p class="text-xs text-muted-foreground">{{ reg.cached_artifact_count }} artifacts</p>
            </div>

            <!-- Last pull -->
            <div class="rounded-lg border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Last pull</p>
              <p class="text-base font-semibold">{{ fmtRelative(reg.last_pull_at) }}</p>
              <p v-if="reg.last_pull_at" class="text-xs text-muted-foreground">{{ fmtDate(reg.last_pull_at) }}</p>
            </div>

            <!-- Pulls / hour -->
            <div class="rounded-lg border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Pulls / hour</p>
              <p
                class="text-xl font-semibold tabular-nums"
                :class="reg.pulls_last_hour > 0 ? 'text-green-600 dark:text-green-400' : 'text-muted-foreground'"
              >
                {{ reg.pulls_last_hour.toLocaleString() }}
              </p>
            </div>

            <!-- Pulls / day -->
            <div class="rounded-lg border bg-muted/30 p-3 space-y-0.5">
              <p class="text-xs text-muted-foreground">Pulls / day</p>
              <p class="text-xl font-semibold tabular-nums">{{ reg.pulls_last_day.toLocaleString() }}</p>
            </div>
          </div>

          <!-- Recent errors -->
          <div>
            <button
              class="flex items-center gap-2 w-full text-left text-sm font-medium py-1"
              @click="toggleErrors(reg.registry)"
            >
              <span v-if="reg.recent_errors.length === 0" class="flex items-center gap-1.5 text-green-600 dark:text-green-400">
                <span class="inline-block h-2 w-2 rounded-full bg-green-500"></span>
                No errors in the last 24 h
              </span>
              <span v-else class="flex items-center gap-1.5 text-orange-600 dark:text-orange-400">
                <span class="inline-block h-2 w-2 rounded-full bg-orange-500"></span>
                {{ reg.recent_errors.length }} error{{ reg.recent_errors.length > 1 ? 's' : '' }} in 24 h
                <span class="text-muted-foreground text-xs ml-auto">
                  {{ expandedErrors.has(reg.registry) ? '▲ hide' : '▼ show' }}
                </span>
              </span>
            </button>

            <div v-if="expandedErrors.has(reg.registry) && reg.recent_errors.length > 0" class="mt-2 rounded-md border overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead class="text-xs">When</TableHead>
                    <TableHead class="text-xs">User</TableHead>
                    <TableHead class="text-xs">Package</TableHead>
                    <TableHead class="text-xs">Type</TableHead>
                    <TableHead class="text-xs">Reason</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  <TableRow v-for="err in reg.recent_errors" :key="err.timestamp + err.package_name">
                    <TableCell class="text-xs whitespace-nowrap">{{ fmtRelative(err.timestamp) }}</TableCell>
                    <TableCell class="text-xs">
                      <span v-if="err.user_id">{{ err.user_id }}</span>
                      <span v-else class="text-muted-foreground italic">anonymous</span>
                    </TableCell>
                    <TableCell class="font-mono text-xs">
                      {{ err.package_name }}<span class="text-muted-foreground">@{{ err.version }}</span>
                    </TableCell>
                    <TableCell>
                      <Badge
                        :variant="err.error_type === 'error' ? 'destructive' : 'secondary'"
                        class="text-xs"
                      >
                        {{ err.error_type === 'error' ? 'Upstream error' : 'Denied' }}
                      </Badge>
                    </TableCell>
                    <TableCell class="text-xs text-muted-foreground max-w-[200px] truncate" :title="err.reason">
                      {{ err.reason }}
                    </TableCell>
                  </TableRow>
                </TableBody>
              </Table>
            </div>
          </div>

          <!-- Who has access -->
          <div class="space-y-1.5">
            <p class="text-xs font-medium text-muted-foreground uppercase tracking-wide">Who has access</p>
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
                v-else-if="!reg.access.roles.includes('anonymous') && !reg.access.roles.includes('user')"
                class="text-xs text-orange-600 dark:text-orange-400 flex items-center gap-1"
              >
                ⚠ Restricted — no public access
              </span>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  </div>

  <!-- Clear cache confirmation dialog -->
  <Dialog :open="clearTarget !== null" @update:open="(v) => { if (!v) { clearTarget = null; clearError = null; } }">
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">Clear cache for <span class="font-mono">{{ clearTarget }}</span>?</h2>
        <p class="text-sm text-muted-foreground mt-1">
          All cached artifacts for this registry will be permanently removed.
          Packages will be re-fetched from upstream on the next request.
        </p>
      </div>
      <p v-if="clearError" class="text-sm text-destructive">{{ clearError }}</p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="clearing"
          @click="clearTarget = null; clearError = null"
        >
          Cancel
        </Button>
        <Button
          variant="destructive"
          size="sm"
          :disabled="clearing"
          @click="confirmClearCache"
        >
          {{ clearing ? "Clearing…" : "Clear Cache" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
