<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useRouter } from "vue-router";
import { Search, Package, RefreshCw } from "@lucide/vue";
import {
  listRegistries,
  exploreRegistryStats,
  explorePackages,
  exploreUpstreamSearch,
} from "@/client/sdk.gen";
import type {
  RegistryInfo,
  RegistryStatDto,
  ExploreEntryDto,
  ExplorePackageListResponse,
  UpstreamPackageDto,
} from "@/client/types.gen";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table";
import { Card, CardContent } from "@/components/ui/card";

// ── Unified row type for the table ────────────────────────────────────────────

type CachedRow = ExploreEntryDto & { kind: "cached" };
type UpstreamRow = UpstreamPackageDto & { kind: "upstream" };
type TableRow = CachedRow | UpstreamRow;

// ── State ─────────────────────────────────────────────────────────────────────

const router = useRouter();

const selectedRegistry = ref<string | null>(null);
const search = ref("");
const sort = ref<"downloads" | "name" | "recent">("downloads");
const page = ref(0);
const perPage = 20;

// All configured accessible registries (sidebar — always complete list)
const allRegistries = ref<RegistryInfo[]>([]);
// Per-registry package counts (only registries that have ≥1 package)
const registryStats = ref<Map<string, RegistryStatDto>>(new Map());

const packages = ref<ExploreEntryDto[]>([]);
const total = ref(0);
const upstreamResults = ref<UpstreamPackageDto[]>([]);

const loading = ref(false);
const loadingRegs = ref(false);
const loadingUpstream = ref(false);
const error = ref<string | null>(null);

// ── Computed ──────────────────────────────────────────────────────────────────

// Merged sidebar list: every registry with its package count (0 if not yet seen)
const sidebarRegistries = computed(() =>
  allRegistries.value.map((r) => ({
    name: r.name,
    package_count: registryStats.value.get(r.name)?.package_count ?? 0,
  })),
);

const totalPackages = computed(() =>
  sidebarRegistries.value.reduce((s, r) => s + r.package_count, 0),
);

// Upstream-only hits (not already cached)
const freshUpstream = computed(() => upstreamResults.value.filter((p) => !p.already_cached));

// Unified rows: cached packages first, then upstream-only hits at the bottom
const tableRows = computed<TableRow[]>(() => [
  ...packages.value.map((p) => ({ ...p, kind: "cached" as const })),
  ...freshUpstream.value.map((p) => ({ ...p, kind: "upstream" as const })),
]);

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / perPage)));

// ── Helpers ───────────────────────────────────────────────────────────────────

function sourceLabel(source: string) {
  if (source === "both") return "Both";
  if (source === "local") return "Local";
  return "Proxied";
}

function sourceVariant(source: string): "default" | "secondary" | "outline" {
  if (source === "local") return "secondary";
  if (source === "both") return "default";
  return "outline";
}

// ── Data fetching ─────────────────────────────────────────────────────────────

async function fetchAllRegistries() {
  loadingRegs.value = true;
  try {
    const [regsResult, statsResult] = await Promise.all([listRegistries(), exploreRegistryStats()]);
    if (regsResult.data) {
      allRegistries.value = (regsResult.data as RegistryInfo[]).sort((a, b) =>
        a.name.localeCompare(b.name),
      );
    }
    if (statsResult.data) {
      const body = statsResult.data as { registries?: RegistryStatDto[] };
      registryStats.value = new Map((body.registries ?? []).map((s) => [s.registry, s]));
    }
  } catch {
    // non-fatal
  } finally {
    loadingRegs.value = false;
  }
}

async function fetchPackages() {
  loading.value = true;
  error.value = null;
  try {
    const { data: res, error: apiErr } = await explorePackages({
      query: {
        page: page.value,
        per_page: perPage,
        sort: sort.value,
        registry: selectedRegistry.value ?? undefined,
        name: search.value.trim() || undefined,
      },
    });
    if (apiErr) throw new Error("Failed to load packages");
    const body = res as ExplorePackageListResponse;
    packages.value = body.items;
    total.value = body.total;
  } catch (e) {
    error.value = e instanceof Error ? e.message : "Failed to load packages";
  } finally {
    loading.value = false;
  }
}

async function fetchUpstream() {
  if (!search.value.trim()) return;
  loadingUpstream.value = true;
  try {
    const { data: res } = await exploreUpstreamSearch({
      query: {
        name: search.value.trim(),
        limit: 10,
        registry: selectedRegistry.value ?? undefined,
      },
    });
    if (res) {
      const body = res as { items?: UpstreamPackageDto[] };
      upstreamResults.value = body.items ?? [];
    }
  } catch {
    // non-fatal
  } finally {
    loadingUpstream.value = false;
  }
}

// ── Actions ───────────────────────────────────────────────────────────────────

let searchTimer: ReturnType<typeof setTimeout> | null = null;
function onSearchInput(val: string) {
  search.value = val;
  if (searchTimer) clearTimeout(searchTimer);
  searchTimer = setTimeout(() => {
    page.value = 0;
    fetchPackages();
    if (val.trim().length >= 2) fetchUpstream();
    else upstreamResults.value = [];
  }, 300);
}

function selectRegistry(reg: string | null) {
  selectedRegistry.value = reg;
  page.value = 0;
  upstreamResults.value = [];
  fetchPackages();
  if (search.value.trim().length >= 2) fetchUpstream();
}

function onSortChange(val: string) {
  sort.value = val as "downloads" | "name" | "recent";
  page.value = 0;
  fetchPackages();
}

function goToDetail(row: TableRow) {
  if (row.kind !== "cached") return;
  router.push({
    path: `/explore/packages/${encodeURIComponent(row.registry)}/${encodeURIComponent(row.name)}`,
  });
}

function prevPage() {
  if (page.value > 0) {
    page.value--;
    fetchPackages();
  }
}
function nextPage() {
  if (page.value < totalPages.value - 1) {
    page.value++;
    fetchPackages();
  }
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

onMounted(() => {
  fetchAllRegistries();
  fetchPackages();
});
</script>

<template>
  <div class="flex gap-6 min-h-[60vh]">
    <!-- Sidebar: full registry list (including those with 0 packages) -->
    <aside class="hidden md:flex flex-col w-56 shrink-0 gap-0.5 border-r border-border/60 pr-4">
      <p class="font-mono text-xs font-semibold text-copper uppercase tracking-wider px-2 mb-2">
        Registries
      </p>

      <button
        :class="[
          'flex items-center justify-between px-2 py-1.5 rounded-sm font-mono text-sm transition-colors w-full text-left',
          selectedRegistry === null
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
        ]"
        @click="selectRegistry(null)"
      >
        <span>All registries</span>
        <Badge variant="outline" class="text-xs ml-1">{{ totalPackages }}</Badge>
      </button>

      <button
        v-for="reg in sidebarRegistries"
        :key="reg.name"
        :class="[
          'flex items-center justify-between px-2 py-1.5 rounded-sm font-mono text-sm transition-colors w-full text-left',
          selectedRegistry === reg.name
            ? 'bg-accent text-accent-foreground font-semibold'
            : reg.package_count === 0
              ? 'text-muted-foreground/50 hover:bg-accent/60 hover:text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
        ]"
        @click="selectRegistry(reg.name)"
      >
        <span class="truncate">{{ reg.name }}</span>
        <Badge
          :variant="reg.package_count === 0 ? 'outline' : 'outline'"
          :class="['text-xs ml-1 shrink-0', reg.package_count === 0 ? 'opacity-40' : '']"
        >
          {{ reg.package_count }}
        </Badge>
      </button>
    </aside>

    <!-- Main content -->
    <div class="flex-1 min-w-0 space-y-4">
      <!-- Header -->
      <div class="flex items-center justify-between gap-4 flex-wrap">
        <h1
          class="font-mono text-xl font-bold flex items-center gap-2 text-foreground cyber-text-glow"
        >
          <Package class="h-5 w-5 text-primary" />
          Package Explorer
        </h1>
        <Button
          variant="outline"
          size="sm"
          @click="
            () => {
              fetchPackages();
              if (search.trim().length >= 2) fetchUpstream();
            }
          "
        >
          <RefreshCw class="h-4 w-4 mr-1" />
          Refresh
        </Button>
      </div>

      <!-- Search + sort bar -->
      <div class="flex gap-2 flex-wrap">
        <div class="relative flex-1 min-w-48">
          <Search class="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            class="pl-8"
            placeholder="Search packages…"
            :value="search"
            @input="onSearchInput(($event.target as HTMLInputElement).value)"
          />
        </div>
        <select
          class="h-9 rounded-sm border border-input bg-background px-3 font-mono text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          :value="sort"
          @change="onSortChange(($event.target as HTMLSelectElement).value)"
        >
          <option value="downloads">Most Downloaded</option>
          <option value="name">Name A–Z</option>
          <option value="recent">Recently Accessed</option>
        </select>
      </div>

      <!-- Error -->
      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

      <!-- Unified table -->
      <Card>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Package</TableHead>
                <TableHead>Registry</TableHead>
                <TableHead class="text-right">Versions</TableHead>
                <TableHead class="text-right">Downloads</TableHead>
                <TableHead>Source</TableHead>
                <TableHead>Proxy</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <template v-if="loading">
                <TableRow>
                  <TableCell colspan="6" class="text-center text-muted-foreground py-8">
                    Loading…
                  </TableCell>
                </TableRow>
              </template>

              <template v-else-if="tableRows.length === 0 && !loadingUpstream">
                <TableRow>
                  <TableCell colspan="6" class="text-center text-muted-foreground py-8">
                    No packages found
                  </TableCell>
                </TableRow>
              </template>

              <!-- Cached packages -->
              <TableRow
                v-for="row in tableRows"
                :key="`${row.kind}-${row.registry}/${row.name}`"
                :class="row.kind === 'cached' ? 'cursor-pointer' : 'cursor-default opacity-70'"
                @click="goToDetail(row)"
              >
                <TableCell class="font-mono text-sm font-medium">{{ row.name }}</TableCell>
                <TableCell>
                  <Badge variant="outline" class="text-xs">{{ row.registry }}</Badge>
                </TableCell>

                <!-- Versions column -->
                <TableCell class="text-right text-sm text-muted-foreground">
                  <template v-if="row.kind === 'cached'">{{ row.version_count }}</template>
                  <span v-else class="italic text-xs">{{ row.latest_version }}</span>
                </TableCell>

                <!-- Downloads column -->
                <TableCell class="text-right text-sm text-muted-foreground">
                  <template v-if="row.kind === 'cached'">
                    {{ row.total_downloads.toLocaleString() }}
                  </template>
                  <span v-else>—</span>
                </TableCell>

                <!-- Source column -->
                <TableCell>
                  <template v-if="row.kind === 'cached'">
                    <Badge :variant="sourceVariant(row.source)" class="text-xs">
                      {{ sourceLabel(row.source) }}
                    </Badge>
                    <Badge v-if="row.has_blocked" variant="destructive" class="text-xs ml-1">
                      Has blocked
                    </Badge>
                  </template>
                  <span v-else class="text-xs text-muted-foreground truncate max-w-[14rem] block">
                    {{ row.description ?? "—" }}
                  </span>
                </TableCell>

                <!-- Proxy status pill -->
                <TableCell>
                  <Badge
                    v-if="row.kind === 'cached'"
                    variant="secondary"
                    class="text-xs whitespace-nowrap"
                  >
                    Proxied
                  </Badge>
                  <Badge
                    v-else
                    variant="outline"
                    class="text-xs whitespace-nowrap border-dashed text-muted-foreground"
                  >
                    Not Yet Proxied
                  </Badge>
                </TableCell>
              </TableRow>

              <!-- Upstream loading indicator -->
              <TableRow v-if="loadingUpstream">
                <TableCell
                  colspan="6"
                  class="text-center text-muted-foreground py-2 text-xs italic"
                >
                  Searching upstream registries…
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <!-- Pagination (cached results only) -->
      <div
        v-if="total > perPage"
        class="flex items-center justify-between text-sm text-muted-foreground"
      >
        <span>{{ total }} cached packages total</span>
        <div class="flex items-center gap-2">
          <Button variant="outline" size="sm" :disabled="page === 0" @click="prevPage">
            Previous
          </Button>
          <span>Page {{ page + 1 }} / {{ totalPages }}</span>
          <Button variant="outline" size="sm" :disabled="page >= totalPages - 1" @click="nextPage">
            Next
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>
