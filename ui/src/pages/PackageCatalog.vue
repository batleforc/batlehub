<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed, onMounted } from "vue";
import { RouterLink, useRouter } from "vue-router";
import { useExploreCache } from "@/composables/useExploreCache";
import { extractMessage } from "@/composables/useApi";
import { formatCount } from "@/lib/format";
import { sourceVariant } from "@/lib/badge-variants";
import { PageHeader } from "@/components/ui/page-header";
import { Facet } from "@/components/ui/facet";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Pagination } from "@/components/ui/pagination";
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

const { t } = useI18n();

// ── Unified row type for the table ────────────────────────────────────────────

type CachedRow = ExploreEntryDto & { kind: "cached" };
type UpstreamRow = UpstreamPackageDto & { kind: "upstream" };
type ExploreRow = CachedRow | UpstreamRow;

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

/** The facet's options, from the same merged list the sidebar rendered. */
const facetOptions = computed(() =>
  sidebarRegistries.value.map((r) => ({
    value: r.name,
    label: r.name,
    count: r.package_count,
  })),
);

const totalPackages = computed(() =>
  sidebarRegistries.value.reduce((s, r) => s + r.package_count, 0),
);

// Upstream-only hits (not already cached)
const freshUpstream = computed(() => upstreamResults.value.filter((p) => !p.already_cached));

// Unified rows: cached packages first, then upstream-only hits at the bottom
const tableRows = computed<ExploreRow[]>(() => [
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

// ── Cache ─────────────────────────────────────────────────────────────────────

interface PageResult {
  items: ExploreEntryDto[];
  total: number;
}
const exploreCache = useExploreCache<PageResult>();

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
  const reg = selectedRegistry.value ?? "";
  const q = search.value.trim();
  const s = sort.value;
  const p = page.value;

  const cached = exploreCache.get(reg, p, s, q);
  if (cached) {
    error.value = null;
    packages.value = cached.items;
    total.value = cached.total;
    return;
  }

  loading.value = true;
  error.value = null;
  try {
    const { data: res, error: apiErr } = await explorePackages({
      query: {
        page: p,
        per_page: perPage,
        sort: s,
        registry: reg || undefined,
        name: q || undefined,
      },
    });
    if (apiErr) throw new Error("Failed to load packages");
    const body = res as ExplorePackageListResponse;
    packages.value = body.items;
    total.value = body.total;
    exploreCache.set(reg, p, s, q, { items: body.items, total: body.total });
  } catch (e) {
    error.value = extractMessage(e);
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
    void fetchPackages();
    if (val.trim().length >= 2) void fetchUpstream();
    else upstreamResults.value = [];
  }, 300);
}

function selectRegistry(reg: string | null) {
  selectedRegistry.value = reg;
  page.value = 0;
  upstreamResults.value = [];
  void fetchPackages();
  if (search.value.trim().length >= 2) void fetchUpstream();
}

function onSortChange(val: string) {
  sort.value = val as "downloads" | "name" | "recent";
  page.value = 0;
  void fetchPackages();
}

function goToDetail(row: ExploreRow) {
  if (row.kind !== "cached") return;
  router.push({
    path: `/packages/${encodeURIComponent(row.registry)}/${encodeURIComponent(row.name)}`,
  });
}

function goToPage(p: number) {
  page.value = p;
  void fetchPackages();
}

// ── Lifecycle ─────────────────────────────────────────────────────────────────

onMounted(() => {
  void fetchAllRegistries();
  void fetchPackages();
});
</script>

<template>
  <div class="flex gap-6 min-h-[60vh]">
    <!-- The registry facet, now the shared primitive rather than 40 lines of
         inline buttons. Selection rides on ink and a lit edge, not a fill. -->
    <aside class="hidden md:block w-56 shrink-0 border-r border-border/60 pr-4">
      <Facet
        :model-value="selectedRegistry"
        :options="facetOptions"
        :label="t('packageCatalog.registries')"
        :all-label="t('packageCatalog.allRegistries', { count: totalPackages })"
        @update:model-value="selectRegistry"
      />
    </aside>

    <!-- Main content -->
    <div class="flex-1 min-w-0 space-y-4">
      <!-- Header -->
      <PageHeader variant="display">
        <template #title>
          <Package class="h-5 w-5 text-primary" />
          {{ t("common.packages") }}
        </template>
        <template #actions>
          <Button
            variant="outline"
            size="sm"
            @click="
              () => {
                exploreCache.invalidate(selectedRegistry ?? undefined);
                void fetchPackages();
                if (search.trim().length >= 2) void fetchUpstream();
              }
            "
          >
            <RefreshCw class="h-4 w-4 mr-1" />
            {{ t("common.refresh") }}
          </Button>
        </template>
      </PageHeader>

      <!-- Search + sort bar -->
      <div class="flex gap-2 flex-wrap">
        <div class="relative flex-1 min-w-48">
          <Search class="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            class="pl-8"
            :placeholder="t('packageCatalog.searchPackages')"
            :aria-label="t('packageCatalog.searchPackages2')"
            :value="search"
            @input="onSearchInput(($event.target as HTMLInputElement).value)"
          />
        </div>
        <select
          :aria-label="t('packageCatalog.sortPackages')"
          class="h-9 rounded-sm border border-input bg-background px-3 font-mono text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
          :value="sort"
          @change="onSortChange(($event.target as HTMLSelectElement).value)"
        >
          <option value="downloads">{{ t("packageCatalog.mostDownloaded") }}</option>
          <option value="name">{{ t("packageCatalog.nameAZ") }}</option>
          <option value="recent">{{ t("packageCatalog.recentlyAccessed") }}</option>
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
                <TableHead>{{ t("common.package") }}</TableHead>
                <TableHead>{{ t("common.registry") }}</TableHead>
                <TableHead class="text-right">{{ t("common.versions") }}</TableHead>
                <TableHead class="text-right">{{ t("common.downloads") }}</TableHead>
                <TableHead>{{ t("common.source") }}</TableHead>
                <TableHead>{{ t("common.proxy") }}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <template v-if="loading">
                <TableRow>
                  <TableCell colspan="6" class="py-4">
                    <Skeleton :lines="6" />
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
                <TableCell class="font-mono text-sm font-medium">
                  {{ row.name }}
                  <!-- One list, provenance stated per row. The question a reader
                       actually has is "does this instance already have it?", and
                       splitting the table into two makes that harder to answer,
                       not easier. -->
                  <span
                    v-if="row.kind === 'upstream'"
                    class="ml-2 font-mono text-xs uppercase tracking-wider text-muted-foreground"
                    >upstream</span
                  >
                </TableCell>
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
                    {{ formatCount(row.total_downloads) }}
                  </template>
                  <span v-else>—</span>
                </TableCell>

                <!-- Source column -->
                <TableCell>
                  <template v-if="row.kind === 'cached'">
                    <Badge :variant="sourceVariant(row.source)" class="text-xs">
                      {{ sourceLabel(row.source) }}
                    </Badge>
                    <Badge v-if="row.has_blocked" variant="destructive" class="text-xs ml-1">{{
                      t("packageCatalog.hasBlocked")
                    }}</Badge>
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
                    {{ t("common.proxied") }}
                  </Badge>
                  <Badge
                    v-else
                    variant="outline"
                    class="text-xs whitespace-nowrap border-dashed text-muted-foreground"
                    >{{ t("packageCatalog.notYetProxied") }}</Badge
                  >
                </TableCell>
              </TableRow>

              <!-- Upstream loading indicator -->
              <TableRow v-if="loadingUpstream">
                <TableCell
                  colspan="6"
                  class="text-center text-muted-foreground py-2 text-xs italic"
                  >{{ t("packageCatalog.searchingUpstreamRegistries") }}</TableCell
                >
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>
      <!-- Empty is two states, and telling them apart is the point: a user
           shown "no packages" while a filter is applied concludes the registry
           is broken.

           Outside the table on purpose: in a `<td colspan>` this inherits the
           table's width, so at 390px the "nothing here" message sat off-screen
           behind a horizontal scroll — the one thing DESIGN.md's Own-Container
           Overflow Rule forbids the body to do. -->
      <div v-if="tableRows.length === 0 && !loadingUpstream && !loading" class="mt-4">
        <EmptyState
          :filtered="Boolean(search.trim())"
          :title="search.trim() ? t('catalog.emptyFilteredTitle') : t('catalog.emptyTitle')"
          :description="search.trim() ? t('catalog.emptyFilteredBody') : t('catalog.emptyBody')"
        >
          <template v-if="search.trim()" #action>
            <Button size="sm" variant="outline" @click="search = ''">{{
              t("packageCatalog.clearSearch")
            }}</Button>
          </template>
          <template v-else #action>
            <Button size="sm" variant="outline" as-child>
              <RouterLink to="/setup">{{ t("packageCatalog.pointAToolAt") }}</RouterLink>
            </Button>
          </template>
        </EmptyState>
      </div>

      <!-- Pagination (cached results only) -->
      <div
        v-if="total > perPage"
        class="flex items-center justify-between text-sm text-muted-foreground"
      >
        <span>{{ t("packageCatalog.cachedPackagesTotal", total) }}</span>
        <Pagination :page="page" :total-pages="totalPages" @update:page="goToPage" />
      </div>
    </div>
  </div>
</template>
