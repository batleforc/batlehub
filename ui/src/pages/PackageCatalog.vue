<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import { RouterLink, useRouter } from "vue-router";
import { useExploreCache, useUpstreamCache } from "@/composables/useExploreCache";
import { extractMessage } from "@/composables/useApi";
import { formatBytes, formatCount, formatRelative } from "@/lib/format";
import { Facet } from "@/components/ui/facet";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Pagination } from "@/components/ui/pagination";
import { Search, RefreshCw } from "@lucide/vue";
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
import { Resolution, type ResolutionState } from "@/components/ui/resolution";
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

const { t } = useI18n();

// ── Unified row type for the table ────────────────────────────────────────────

type CachedRow = ExploreEntryDto & { kind: "cached" };
type UpstreamRow = UpstreamPackageDto & { kind: "upstream" };
type ExploreRow = CachedRow | UpstreamRow;

// ── State ─────────────────────────────────────────────────────────────────────

const router = useRouter();

const selectedRegistry = ref<string | null>(null);
const search = ref("");
/**
 * Where the search looks: names, README prose, or both (RFC 0007-bis §4.3).
 *
 * `name` is what this page has always done, and it stays the default — the
 * control is an opt-in to a *wider* search, never a narrowing of the one people
 * already know.
 */
type SearchScope = "name" | "readme" | "both";
const searchIn = ref<SearchScope>("name");
/** `[search] readmes` on this instance, as the last response reported it. */
const readmeSearchEnabled = ref(false);
/** The prose search hit its cap; there may be more than is shown. */
const searchTruncated = ref(false);

type SortKey = "fetched" | "downloads" | "name" | "recent";

/* The proof's catalog is ordered by last fetch, and says so in its caption.
   That was not an option here — the API offered downloads/name/recent, where
   `recent` is when a client last downloaded *from* us, a different fact — so
   `fetched` was added alongside it rather than by redefining `recent`. It is
   the default for the same reason the proof chooses it: a catalog answers
   "what has been moving through here lately" before it answers "what is
   popular". */
const sort = ref<SortKey>("fetched");
const page = ref(0);
const perPage = 20;

// All configured accessible registries (sidebar — always complete list)
const allRegistries = ref<RegistryInfo[]>([]);
// Per-registry package counts (only registries that have ≥1 package)
const registryStats = ref<Map<string, RegistryStatDto>>(new Map());
/** True when the counts could not be read at all — not when they are zero. */
const statsUnavailable = ref(false);

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
    count: countsKnown.value ? r.package_count : undefined,
  })),
);

const totalPackages = computed(() =>
  sidebarRegistries.value.reduce((s, r) => s + r.package_count, 0),
);

/** Counts are shown only when they were actually read. */
const countsKnown = computed(() => !statsUnavailable.value);

// ── The specimen (RFC 0004-bis §14.9) ────────────────────────────────────────
//
// `ui/design-proof/index.html` spends the Display step on **the registry you
// are looking at**, so the page announces its subject. This page spent 24px on
// the word "Packages" — a label on a door — because `--t-display` was mapped to
// no utility and `text-2xl` was the largest step that existed.
//
// The subject is the selected registry, or the instance itself when the facet
// is on "all": "every registry" is still an answer to "what am I looking at",
// and a blank specimen would be the §2.4 defect in a headline.

/** The registry whose name the Display step carries, or null for "all". */
const specimenRegistry = computed(() =>
  selectedRegistry.value
    ? (allRegistries.value.find((r) => r.name === selectedRegistry.value) ?? null)
    : null,
);

/**
 * The caption's facts, in the proof's order — *"hybrid · registry.npmjs.org ·
 * 1,284 packages · 6.1 GB cached"*: how it runs, what is behind it, how much of
 * it there is, how much of it we hold.
 *
 * The last two used to be missing outright. `upstream` was not on
 * `/api/v1/registries` at all, and the cached size was not on any endpoint this
 * page calls — a previous pass noted it and left the fact out rather than
 * estimate it, which was the right call and the wrong resting place. Both are
 * now served, so the caption is the proof's four facts rather than two of them.
 *
 * Still only what the API actually returned: each fact is pushed on its own
 * condition, so a registry with no upstream (local mode) or an instance whose
 * stats query failed drops that item instead of printing a zero.
 */
const specimenFacts = computed<string[]>(() => {
  const reg = specimenRegistry.value;
  const facts: string[] = [];
  if (reg) {
    facts.push(reg.type, reg.mode);
    if (reg.upstream) facts.push(reg.upstream);
    if (countsKnown.value) {
      const stat = registryStats.value.get(reg.name);
      facts.push(
        t("packageCatalog.packageCount", {
          count: formatCount(stat?.package_count ?? 0),
        }),
      );
      // `null` is "we never recorded sizes", which is not "0 B".
      if (stat?.cached_bytes != null) {
        facts.push(t("packageCatalog.cachedSize", { size: formatBytes(stat.cached_bytes) }));
      }
    }
  } else if (countsKnown.value) {
    facts.push(
      t("packageCatalog.registryCount", { count: allRegistries.value.length }),
      t("packageCatalog.packageCount", { count: formatCount(totalPackages.value) }),
    );
    if (totalCachedBytes.value != null) {
      facts.push(t("packageCatalog.cachedSize", { size: formatBytes(totalCachedBytes.value) }));
    }
  }
  return facts;
});

/** Summed only over the registries that reported a size; `null` when none did,
    so "nobody recorded any sizes" never renders as "0 B held". */
const totalCachedBytes = computed<number | null>(() => {
  let total: number | null = null;
  for (const stat of registryStats.value.values()) {
    if (stat.cached_bytes != null) total = (total ?? 0) + stat.cached_bytes;
  }
  return total;
});

/**
 * The table's caption, which the proof spends on the two things a reader needs
 * before scanning a column of rows: how many there are, and what order they are
 * in. The ordering half is not decoration — a table sorted by last fetch and a
 * table sorted by downloads look identical, and the proof's own caption is what
 * tells them apart.
 *
 * Named from the same catalogue entries as the sort control, so the caption
 * cannot describe an order the control does not offer.
 */
const SORT_LABEL_KEYS: Record<SortKey, string> = {
  fetched: "packageCatalog.lastFetch",
  downloads: "packageCatalog.mostDownloaded",
  name: "packageCatalog.nameAZ",
  recent: "packageCatalog.recentlyAccessed",
};

const tableCaption = computed(() =>
  t("packageCatalog.tableCaption", {
    count: formatCount(total.value),
    sort: t(SORT_LABEL_KEYS[sort.value]),
  }),
);

// Upstream-only hits (not already cached)
const freshUpstream = computed(() => upstreamResults.value.filter((p) => !p.already_cached));

// Unified rows: cached packages first, then upstream-only hits at the bottom
const tableRows = computed<ExploreRow[]>(() => [
  ...packages.value.map((p) => ({ ...p, kind: "cached" as const })),
  ...freshUpstream.value.map((p) => ({ ...p, kind: "upstream" as const })),
]);

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / perPage)));

// ── Resolution as state ───────────────────────────────────────────────────────
//
// DESIGN.md's organising idea, and the reason this page exists in the form it
// does: what BatleHub holds and has verified renders at full resolution, what it
// does not renders coarse. The six states are named by the *server* — two of
// them need the registry's artifact TTL and the release-age gate's window and
// bypass roles, and `held` depends on who is asking — so this file's job is to
// map a string onto a mark and a word, not to decide anything.

const RESOLUTION_STATES = [
  "cached",
  "stale",
  "held",
  "pending",
  "yanked",
  "blocked",
] as const satisfies readonly ResolutionState[];

/** Spelled out, not `t(\`resolution.${s}\`)`: a template-literal key is invisible
    to the catalogue's reference gate, which is how 94 keys drifted out of use
    with everything green (RFC 0004-bis §2.2). */
const STATE_LABEL_KEYS: Record<ResolutionState, string> = {
  cached: "resolution.cached",
  stale: "resolution.stale",
  held: "resolution.held",
  pending: "resolution.pending",
  yanked: "resolution.yanked",
  blocked: "resolution.blocked",
};

/**
 * The three states that are a refusal, and the rule behind each.
 *
 * The proof states a denial's rule *in its own row*, tied to the package by
 * `aria-describedby`, rather than floating one note above the table — so the
 * reason travels with the thing it is about, for a screen reader as well as for
 * a reader scanning the column.
 *
 * The wording names the mechanism and stops there. The proof's notes quote
 * specifics ("published 11 minutes ago, held until 24 h"); this listing does not
 * carry the block reason, the yank date or the quarantine's remaining window —
 * those live on the package's own page, which each note links to. Stating a
 * mechanism we know beats interpolating numbers we do not have.
 */
const NOTE_KEYS: Partial<Record<ResolutionState, string>> = {
  blocked: "resolution.noteBlocked",
  yanked: "resolution.noteYanked",
  held: "resolution.noteHeld",
};

/** An upstream search hit has never been fetched, which is `pending`'s
    definition; a cached row carries whatever the server graded it. */
function rowState(row: ExploreRow): ResolutionState {
  if (row.kind !== "cached") return "pending";
  return (RESOLUTION_STATES as readonly string[]).includes(row.state)
    ? (row.state as ResolutionState)
    : "pending";
}

function noteKey(row: ExploreRow): string | undefined {
  return NOTE_KEYS[rowState(row)];
}

/**
 * Why this row is here, or `undefined` when it needs no saying.
 *
 * Only a prose match is labelled. A row that matched on its name is
 * self-explanatory, and labelling every row would make the one that actually
 * needs the label invisible (RFC 0007-bis §4.3).
 */
function matchLabel(row: ExploreRow): string | undefined {
  if (row.kind !== "cached") return undefined;
  const matched = (row as ExploreEntryDto).matched_in;
  if (matched === "readme") return t("packageCatalog.matchedReadme");
  if (matched === "both") return t("packageCatalog.matchedBoth");
  return undefined;
}

/** The matched fragment of a README, as plain text. Never markup. */
function rowSnippet(row: ExploreRow): string | undefined {
  if (row.kind !== "cached") return undefined;
  return (row as ExploreEntryDto).snippet ?? undefined;
}

/**
 * A prose search actually ran and came back with nothing.
 *
 * Distinct from "the filter matched nothing": the empty state has to say what
 * was searched, and `in=readme` with the feature *off* answers as a name search
 * — so the scope alone is not enough to decide which words to show.
 */
const searchedProse = computed(
  () => readmeSearchEnabled.value && searchIn.value !== "name" && search.value.trim().length > 0,
);

/** Stable across re-renders because it is derived from the row's identity, not
    from its index: `aria-describedby` pointing at a recycled id is worse than
    pointing at nothing. */
function rowId(row: ExploreRow): string {
  return `${row.kind}-${row.registry}/${row.name}`;
}

// ── Cache ─────────────────────────────────────────────────────────────────────

interface PageResult {
  items: ExploreEntryDto[];
  total: number;
}
const exploreCache = useExploreCache<PageResult>();
const upstreamCache = useUpstreamCache<UpstreamPackageDto[]>();

/**
 * Sequence guards (RFC 0004-bis §6.3).
 *
 * Neither fetch guarded its own ordering: `packages.value = body.items` was
 * unconditional, with no sequence token and no `AbortController`, and
 * `selectRegistry`/`onSortChange` are undebounced. So clicking registry A
 * (uncached, slow) then B (cached, instant) let A's response overwrite the
 * table while the sidebar showed B selected. The cache entries stayed correct
 * throughout — it is only the *display* that was wrong, which is why the fix is
 * a sequence number rather than a change to the keys.
 *
 * A late response is still written to the cache and not to the screen. That is
 * deliberate: the response is correct for its own key, it is only stale for
 * what is currently being looked at, and discarding it would mean re-fetching
 * it the moment the operator clicks back.
 */
let packagesSeq = 0;
let upstreamSeq = 0;

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
      const body = statsResult.data as {
        registries?: RegistryStatDto[];
        upstream_unavailable?: boolean;
      };
      registryStats.value = new Map((body.registries ?? []).map((s) => [s.registry, s]));
      // The server answers `{ registries: [], upstream_unavailable: true }`
      // when the stats query fails, and this page discarded the flag — so a
      // failed query rendered as every registry showing 0, indistinguishable
      // from an instance that genuinely holds nothing. Counts unknown and
      // counts zero are different facts.
      statsUnavailable.value = body.upstream_unavailable === true;
    } else if (statsResult.error) {
      statsUnavailable.value = true;
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
  const scope = searchIn.value;
  const seq = ++packagesSeq;

  const cached = exploreCache.get(reg, p, s, q, scope);
  if (cached) {
    error.value = null;
    packages.value = cached.items;
    total.value = cached.total;
    // `loading` has to be cleared here too, not only in the `finally` below.
    // This call already bumped `packagesSeq`, so any request still in flight
    // will fail its own `seq === packagesSeq` check and skip the `finally` —
    // leaving the skeleton row on screen forever. Reproduced by selecting an
    // uncached registry and then a cached one before the first lands.
    loading.value = false;
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
        // `q` is the parameter `in` applies to; `name` is the older one and
        // still filters names, which is why the scope is sent with `q` alone.
        q: q || undefined,
        in: scope,
      },
    });
    if (apiErr) throw new Error(t("packageCatalog.loadFailed"));
    const body = res as ExplorePackageListResponse;
    // Written under the coordinates captured at call time, not the current
    // refs, so a late response cannot land under the wrong key.
    exploreCache.set(reg, p, s, q, { items: body.items, total: body.total }, scope);
    if (seq !== packagesSeq) return; // superseded — cached, not displayed
    packages.value = body.items;
    total.value = body.total;
    // Reported by the server rather than inferred: a client cannot tell "no
    // package here says that" from "this instance does not search prose", and
    // guessing would put the wrong empty state on screen.
    readmeSearchEnabled.value = body.readme_search_enabled === true;
    searchTruncated.value = body.truncated === true;
  } catch (e) {
    if (seq === packagesSeq) error.value = extractMessage(e);
  } finally {
    if (seq === packagesSeq) loading.value = false;
  }
}

async function fetchUpstream() {
  const name = search.value.trim();
  if (!name) return;
  const reg = selectedRegistry.value ?? "";
  const seq = ++upstreamSeq;

  // Every hit here is N third-party calls that do not happen.
  const cached = upstreamCache.get(name, reg);
  if (cached) {
    upstreamResults.value = cached;
    return;
  }

  loadingUpstream.value = true;
  try {
    const { data: res } = await exploreUpstreamSearch({
      query: { name, limit: 10, registry: reg || undefined },
    });
    if (res) {
      const body = res as { items?: UpstreamPackageDto[] };
      const items = body.items ?? [];
      upstreamCache.set(name, reg, items);
      if (seq !== upstreamSeq) return; // superseded — cached, not displayed
      upstreamResults.value = items;
    }
  } catch {
    // non-fatal
  } finally {
    if (seq === upstreamSeq) loadingUpstream.value = false;
  }
}

// ── Actions ───────────────────────────────────────────────────────────────────

let searchTimer: ReturnType<typeof setTimeout> | null = null;

// Routing away within 300 ms of a keystroke otherwise ran `fetchPackages` — and
// possibly `fetchUpstream` — against a destroyed component. The tests never saw
// it because `typeSearch` always waits 350 ms, past the deadline.
onBeforeUnmount(() => {
  if (searchTimer) clearTimeout(searchTimer);
});

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

/**
 * Changing the scope re-runs the search at once, undebounced.
 *
 * The query has not changed — only what it is asked of — so there is nothing to
 * wait for, and a delay after a select would read as the control not working.
 */
function onSearchInChange(val: string) {
  searchIn.value = (["name", "readme", "both"] as const).includes(val as SearchScope)
    ? (val as SearchScope)
    : "name";
  page.value = 0;
  void fetchPackages();
}

function selectRegistry(reg: string | null) {
  selectedRegistry.value = reg;
  page.value = 0;
  upstreamResults.value = [];
  void fetchPackages();
  if (search.value.trim().length >= 2) void fetchUpstream();
}

function onSortChange(val: string) {
  sort.value = val as SortKey;
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
  <div>
    <!-- The specimen. Not `PageHeader`: this is the one surface with a
         checked-in statement of its own appearance, and that statement gives
         the Display step to the subject rather than to the page's nav label
         (RFC 0004-bis §2.7, §14.9). Every other page keeps PageHeader.

         Full width, above the sheet — the proof's own arrangement, where
         `.specimen` is a direct child of `<main>` and `.sheet` starts beneath
         it. It used to sit *inside* the right-hand column, which meant the bay
         ran alongside it and the display type began 232px in from the page
         edge. A specimen is the page announcing its subject; starting it
         indented, in the gutter of a list it is not part of, is the one
         placement that undercuts that.

         `overflow-hidden` contains the plate; `relative` gives it a frame.
         The h1 sits above it on z-index — the plate is `aria-hidden` and
         purely decorative.

         The bottom rule is `--rule-soft`, the separator token, rather than
         `--border` (`--rule-strong`): it now divides the specimen from the
         sheet across the whole window, which is the job the proof gives
         `.specimen{border-bottom:1px solid var(--rule-soft)}`. At full width a
         contrast-carrying weight reads as a second masthead. -->
    <section class="relative overflow-hidden border-b border-rule-soft pb-6 pt-4">
      <div class="plate" aria-hidden="true"></div>
      <!-- The proof's `.display` rule, ported rather than approximated:
           `line-height:.92`, `letter-spacing:.02em`, uppercase. The body
           line-height is 1.625, which on a 104px element costs 169px per line
           — a two-line registry name came to 312px of headline before the
           measurement caught it. A display face is set tight; that is most of
           what makes it read as display rather than as a very large heading. -->
      <h1
        class="relative z-10 font-display font-bold uppercase tracking-[0.02em] leading-[0.92] text-display break-words"
        data-testid="specimen-name"
      >
        {{ specimenRegistry?.name ?? t("packageCatalog.allRegistriesTitle") }}
      </h1>
      <p
        v-if="specimenFacts.length"
        class="relative z-10 mt-3 border-t border-border pt-3 text-sm text-muted-foreground max-w-[72ch]"
      >
        <template v-for="(fact, i) in specimenFacts" :key="fact">
          <span v-if="i > 0" class="px-2 text-border" aria-hidden="true">·</span>
          <span class="text-foreground">{{ fact }}</span>
        </template>
      </p>
    </section>

    <!-- The sheet: the proof's `grid-template-columns: 232px 1fr` with
         `align-items:start`, so the bay is its own column beside the catalog
         and neither stretches to match the other's height. Below `md` it
         collapses to one column, as the proof collapses it at 900px. -->
    <div class="grid min-h-[60vh] items-start md:grid-cols-[14rem_1fr]">
      <!-- The registry bay, the shared facet primitive rather than 40 lines of
           inline buttons. Selection rides on ink and a lit edge, not a fill.
           The separator is `--rule-soft` — it divides two regions, it does not
           carry contrast, and the proof rules it accordingly. -->
      <!-- Visible at every width. It used to be `hidden md:block`, which left a
           phone with no way to change registry at all — the filter existed and
           its only control did not. Below `md` it is a rule *under* the strip
           rather than beside it, matching the orientation the cells take. -->
      <!-- `min-w-0` for the same reason the catalog column below needs it, and
           it bites harder here: a grid item's default `min-width:auto` refuses
           to shrink below its content's min-content width, and this item's
           content is a nowrap flex row of every registry. Without it that row's
           full width propagates out through the grid and the *body* scrolls
           sideways — the one thing the Own-Container Overflow Rule forbids.
           With it, the row scrolls inside itself, which is the point. -->
      <aside
        class="min-w-0 border-b border-rule-soft py-3 md:border-b-0 md:border-r md:py-6 md:pr-4"
      >
        <Facet
          :model-value="selectedRegistry"
          :options="facetOptions"
          :label="t('packageCatalog.registries')"
          :all-label="
            countsKnown
              ? t('packageCatalog.allRegistries', { count: totalPackages })
              : t('packageCatalog.allRegistriesUnknown')
          "
          @update:model-value="selectRegistry"
        />
      </aside>

      <!-- `min-w-0` is load-bearing on a grid child: without it the track
           refuses to shrink below its content's min-content width, and the
           table's own scroll container never gets the chance to do its job. -->
      <div class="min-w-0 space-y-4 py-6 md:pl-6">
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
          <!-- Where the search looks. Beside the box it modifies rather than in
             a settings panel, because it changes what the *next keystroke*
             means. Hidden entirely when the instance searches names only: a
             control whose other options do nothing is worse than no control
             (RFC 0007-bis §4.3). -->
          <select
            v-if="readmeSearchEnabled"
            :aria-label="t('packageCatalog.searchIn')"
            class="h-9 rounded-sm border border-input bg-background px-3 font-mono text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            :value="searchIn"
            @change="onSearchInChange(($event.target as HTMLSelectElement).value)"
          >
            <option value="name">{{ t("packageCatalog.searchInName") }}</option>
            <option value="readme">{{ t("packageCatalog.searchInReadme") }}</option>
            <option value="both">{{ t("packageCatalog.searchInBoth") }}</option>
          </select>
          <!-- The page's one action, in the toolbar beside search — where
             `design-proof/index.html` puts its own (RFC 0004-bis §14.9). It
             used to sit in `PageHeader`, which the specimen replaced. -->
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
          <select
            :aria-label="t('packageCatalog.sortPackages')"
            class="h-9 rounded-sm border border-input bg-background px-3 font-mono text-sm text-foreground focus:outline-none focus:ring-2 focus:ring-ring"
            :value="sort"
            @change="onSortChange(($event.target as HTMLSelectElement).value)"
          >
            <option value="fetched">{{ t("packageCatalog.lastFetch") }}</option>
            <option value="downloads">{{ t("packageCatalog.mostDownloaded") }}</option>
            <option value="name">{{ t("packageCatalog.nameAZ") }}</option>
            <option value="recent">{{ t("packageCatalog.recentlyAccessed") }}</option>
          </select>
        </div>

        <!-- Error -->
        <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

        <!-- The catalog.

           No `Card`. The proof puts the table straight onto the sheet, and the
           card was adding a second boundary around content the ruled header row
           already bounds — two frames around one table, on a system whose
           Flat-At-Rest Rule allows no elevation to justify either.

           Column order is the proof's, and the order is the argument: State
           leads, because "does this instance hold it" is the question the whole
           world is organised around, and a reader scanning for what is wrong
           should not have to cross five columns to find out. It used to sit
           last, behind Registry, Versions, Downloads and Source. -->
        <Table :label="t('packageCatalog.tableLabel')">
          <!-- Body, not Meta. DESIGN.md gives uppercase-and-tracked to labels —
               column heads, state words, nav — and prose to captions, and this
               is a sentence: "7 packages · sorted by last fetch" is 33
               characters of it. Set in caps it was the one run of body text on
               the page that the all-caps rule catches, because caps strips the
               ascenders and descenders a reader recognises words by. The
               tracking goes with it: letterspacing is a small-caps device. -->
          <caption class="caption-top pb-3 text-left text-xs text-muted-foreground">
            {{
              tableCaption
            }}
          </caption>
          <TableHeader>
            <!-- The one solid rule in the table, under the header: the proof
               separates the head from the body with `--rule-strong` and every
               row below with a dashed `--rule-soft`. One weight for "this is
               the boundary", a lighter dashed one for "these are the divisions
               inside it". -->
            <!-- Cells are padded on the right and flush left, per the proof's
               `padding: --s3 --s3 --s3 0`: the column gap belongs *between*
               columns, and a left-flush first column is what lines the table up
               with the caption and the specimen rule above it.

               The fixed widths only apply from `md`. Below it they are released
               exactly as the proof releases them (`.c-state,.c-ver{width:auto}`
               under 900px) — 9.5rem of State inside a 358px box leaves the
               package name nowhere to go, and the columns collide. -->
            <TableRow class="border-b border-solid border-border hover:bg-transparent">
              <TableHead class="pl-0 pr-3 md:w-[9.5rem]">{{ t("common.state") }}</TableHead>
              <TableHead class="pl-0 pr-3">{{ t("common.package") }}</TableHead>
              <TableHead class="pl-0 pr-3 md:w-[9rem]">{{ t("common.version") }}</TableHead>
              <!-- Dropped below `md`, exactly as the proof drops them: at 390px
                 the fixed columns alone claimed 260px of a 358px box, and the
                 two numeric ones are the least load-bearing. -->
              <TableHead class="hidden pl-0 pr-3 text-right md:table-cell md:w-24">{{
                t("common.size")
              }}</TableHead>
              <TableHead class="hidden px-0 text-right md:table-cell md:w-32">{{
                t("packageCatalog.lastFetch")
              }}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-if="loading" class="border-b-0 hover:bg-transparent">
              <TableCell colspan="5" class="px-0 py-4">
                <Skeleton :lines="6" />
              </TableCell>
            </TableRow>

            <template v-for="row in tableRows" :key="rowId(row)">
              <TableRow
                :class="[
                  'border-dashed border-rule-soft align-baseline hover:border-solid hover:border-border',
                  row.kind === 'cached' ? 'cursor-pointer' : 'cursor-default',
                  noteKey(row) ? 'border-b-0' : '',
                ]"
                @click="goToDetail(row)"
              >
                <TableCell class="pl-0 pr-3 py-3 align-baseline">
                  <Resolution :state="rowState(row)" :label="t(STATE_LABEL_KEYS[rowState(row)])" />
                </TableCell>
                <!-- `aria-describedby` is what ties a denial to the package it is
                   about. Without it the note below is a paragraph a screen
                   reader meets after the row and has to relate by proximity.

                   `overflow-wrap:anywhere` is what the proof sets on `.pkg` —
                   not `break-words`. The two differ in exactly the way that
                   matters here: `anywhere` is taken into account when the
                   browser computes the cell's *min-content* width, `break-word`
                   is not. So with `break-words` a name like
                   `github.com/spf13/cobra` still demanded its full width, the
                   table grew past the viewport, and the whole catalog sat
                   behind a horizontal scrollbar at 390px. -->
                <TableCell
                  class="[overflow-wrap:anywhere] pl-0 pr-3 py-3 align-baseline text-base"
                  :aria-describedby="noteKey(row) ? `note-${rowId(row)}` : undefined"
                >
                  {{ row.name }}
                  <!-- One list, provenance stated per row. The question a reader
                     actually has is "does this instance already have it?", and
                     splitting the table into two makes that harder to answer,
                     not easier. -->
                  <span
                    v-if="row.kind === 'upstream'"
                    class="ml-2 text-xs uppercase tracking-[0.1em] text-muted-foreground"
                    >{{ t("packageCatalog.upstreamRow") }}</span
                  >
                  <span
                    v-else-if="!selectedRegistry"
                    class="ml-2 text-xs uppercase tracking-[0.1em] text-muted-foreground"
                    >{{ row.registry }}</span
                  >
                  <!-- Why this row is here. Shown only for a prose match: a row
                     that matched on its name needs no explanation, and labelling
                     every row would make the one that needs it invisible. -->
                  <span
                    v-if="matchLabel(row)"
                    class="ml-2 text-xs uppercase tracking-[0.1em] text-muted-foreground"
                    :title="t('packageCatalog.matchedInLabel', { where: matchLabel(row) })"
                    >{{ matchLabel(row) }}</span
                  >
                </TableCell>
                <TableCell class="pl-0 pr-3 py-3 align-baseline text-muted-foreground tabular-nums">
                  <template v-if="row.kind === 'cached'">{{ row.newest_version ?? "—" }}</template>
                  <template v-else>{{ row.latest_version ?? "—" }}</template>
                </TableCell>
                <TableCell
                  class="hidden pl-0 pr-3 py-3 text-right align-baseline text-muted-foreground tabular-nums md:table-cell"
                >
                  <template v-if="row.kind === 'cached'">{{
                    formatBytes(row.cached_bytes)
                  }}</template>
                  <template v-else>—</template>
                </TableCell>
                <TableCell
                  class="hidden whitespace-nowrap px-0 py-3 text-right align-baseline text-muted-foreground tabular-nums md:table-cell"
                >
                  <template v-if="row.kind === 'cached'">{{
                    formatRelative(row.last_fetched_at, { fallback: "—" })
                  }}</template>
                  <template v-else>—</template>
                </TableCell>
              </TableRow>

              <!-- Every denial states its rule in its own row, tied to the
                 package by `aria-describedby` — not one note floating above the
                 table. The rule is indented under the package rather than under
                 the state, so the eye reads it as belonging to the name. -->
              <!-- The matched fragment, as **text**. It is interpolated, never
                 `v-html`: the README panel's markup boundary is a deliberate,
                 tested, single-component one, and a search snippet is a second
                 surface for package-authored content reached by a much cheaper
                 path — no navigation, just a query (RFC 0007-bis §7.4). -->
              <TableRow
                v-if="rowSnippet(row)"
                class="border-dashed border-rule-soft hover:bg-transparent"
              >
                <TableCell class="pl-0 pr-3 pb-3 pt-0" />
                <TableCell colspan="4" class="pl-0 pr-3 pb-3 pt-0">
                  <p class="flex items-stretch gap-3 text-sm text-muted-foreground">
                    <span class="w-px flex-none bg-border" aria-hidden="true" />
                    <span class="min-w-0 [overflow-wrap:anywhere]">{{ rowSnippet(row) }}</span>
                  </p>
                </TableCell>
              </TableRow>

              <TableRow
                v-if="noteKey(row)"
                class="border-dashed border-rule-soft hover:bg-transparent"
              >
                <TableCell class="pl-0 pr-3 pb-3 pt-0" />
                <TableCell colspan="2" class="pl-0 pr-3 pb-3 pt-0">
                  <p
                    :id="`note-${rowId(row)}`"
                    class="flex items-stretch gap-3 text-sm text-muted-foreground"
                  >
                    <span class="w-px flex-none bg-border" aria-hidden="true" />
                    <span class="min-w-0 [overflow-wrap:anywhere]">
                      {{ t(noteKey(row)!) }}
                      <!-- Underlined at rest, not on hover. Crimson on the
                           note's dim ink measures 1.28:1 against the surrounding
                           text, so colour alone cannot mark it as a link — axe's
                           `link-in-text-block` wants 3:1 or a non-colour cue,
                           and the design gate failed on exactly that. The proof
                           underlines it too: its links take the browser default
                           and it only sets the offset. -->
                      <RouterLink
                        v-if="row.kind === 'cached'"
                        class="text-primary underline underline-offset-[3px]"
                        :to="`/packages/${encodeURIComponent(row.registry)}/${encodeURIComponent(row.name)}`"
                        @click.stop
                        >{{ t("packageCatalog.noteDetails") }}</RouterLink
                      >
                    </span>
                  </p>
                </TableCell>
                <TableCell class="hidden px-0 pb-3 pt-0 md:table-cell" />
                <TableCell class="hidden px-0 pb-3 pt-0 md:table-cell" />
              </TableRow>
            </template>

            <!-- Upstream loading indicator -->
            <TableRow v-if="loadingUpstream" class="border-b-0 hover:bg-transparent">
              <TableCell colspan="5" class="px-0 py-2 text-center text-xs text-muted-foreground">{{
                t("packageCatalog.searchingUpstreamRegistries")
              }}</TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <!-- A cap that applied, said out loud. A silently shortened list reads
             as "that is all there is", which is a lie about the catalogue. -->
        <p v-if="searchTruncated" class="mt-2 text-xs text-muted-foreground">
          {{ t("packageCatalog.readmeSearchTruncated", { count: total }) }}
        </p>

        <!-- Empty is two states, and telling them apart is the point: a user
           shown "no packages" while a filter is applied concludes the registry
           is broken.

           Outside the table on purpose: in a `<td colspan>` this inherits the
           table's width, so at 390px the "nothing here" message sat off-screen
           behind a horizontal scroll — the one thing DESIGN.md's Own-Container
           Overflow Rule forbids the body to do. -->
        <div v-if="tableRows.length === 0 && !loadingUpstream && !loading" class="mt-4">
          <!-- A prose search that found nothing gets its own words. "Nothing
             matches that search" would imply the query was checked against every
             package here, and it was checked against the READMEs of the versions
             this instance holds — which is a narrower claim and the honest one
             (RFC 0007-bis §4.3). -->
          <EmptyState
            :filtered="Boolean(search.trim())"
            :title="
              searchedProse
                ? t('packageCatalog.readmeSearchEmptyTitle')
                : search.trim()
                  ? t('catalog.emptyFilteredTitle')
                  : t('catalog.emptyTitle')
            "
            :description="
              searchedProse
                ? t('packageCatalog.readmeSearchEmptyBody')
                : search.trim()
                  ? t('catalog.emptyFilteredBody')
                  : t('catalog.emptyBody')
            "
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
  </div>
</template>
